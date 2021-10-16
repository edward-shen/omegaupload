#![warn(clippy::nursery, clippy::pedantic)]

use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use axum::http::StatusCode;
use paste::Expiration;
use rand::prelude::StdRng;
use rand::{Rng, SeedableRng};
use rocksdb::IteratorMode;
use rocksdb::WriteBatch;
use rocksdb::{Options, DB};
use short_code::ShortCode;

use anyhow::Result;
use axum::body::Bytes;
use axum::extract::{Extension, Path, TypedHeader};
use axum::handler::{get, post};
use axum::{AddExtensionLayer, Router};
use tokio::task;
use tracing::warn;
use tracing::{error, instrument};

use crate::paste::Paste;
use crate::time::FIVE_MINUTES;

mod paste;
mod short_code;
mod time;

#[tokio::main]
async fn main() -> Result<()> {
    const DB_PATH: &str = "database";
    const SHORT_CODE_SIZE: usize = 12;

    tracing_subscriber::fmt::init();

    let db = Arc::new(DB::open_default(DB_PATH)?);

    let stop_signal = Arc::new(AtomicBool::new(false));
    task::spawn(cleanup(Arc::clone(&stop_signal), Arc::clone(&db)));

    axum::Server::bind(&"0.0.0.0:8080".parse()?)
        .serve(
            Router::new()
                .route("/", post(upload::<SHORT_CODE_SIZE>))
                .route(
                    "/:code",
                    get(paste::<SHORT_CODE_SIZE>).delete(delete::<SHORT_CODE_SIZE>),
                )
                .layer(AddExtensionLayer::new(db))
                .layer(AddExtensionLayer::new(StdRng::from_entropy()))
                .into_make_service(),
        )
        .await?;

    stop_signal.store(true, Ordering::Release);
    // Must be called for correct shutdown
    DB::destroy(&Options::default(), DB_PATH)?;
    Ok(())
}

#[instrument(skip(db), err)]
async fn upload<const N: usize>(
    Extension(db): Extension<Arc<DB>>,
    Extension(mut rng): Extension<StdRng>,
    maybe_expires: Option<TypedHeader<Expiration>>,
    body: Bytes,
) -> Result<Vec<u8>, StatusCode> {
    if body.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }

    // 3GB max; this is a soft-limit of RocksDb
    if body.len() >= 3_221_225_472 {
        return Err(StatusCode::PAYLOAD_TOO_LARGE);
    }

    let paste = Paste::new(maybe_expires.map(|v| v.0).unwrap_or_default(), body);
    let mut new_key = None;

    // Try finding a code; give up after 1000 attempts
    // Statistics show that this is very unlikely to happen
    for _ in 0..1000 {
        let code: ShortCode<N> = rng.sample(short_code::Generator);
        let db = Arc::clone(&db);
        let key = code.as_bytes();
        let query = task::spawn_blocking(move || db.key_may_exist(key)).await;
        if matches!(query, Ok(false)) {
            new_key = Some(key);
        }
    }

    let key = if let Some(key) = new_key {
        key
    } else {
        error!("Failed to generate a valid shortcode");
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    };

    let value = if let Ok(v) = bincode::serialize(&paste) {
        v
    } else {
        error!("Failed to serialize paste?!");
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    };

    match task::spawn_blocking(move || db.put(key, value)).await {
        Ok(Ok(_)) => (),
        e => {
            error!("Failed to insert paste into db: {:?}", e);
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    }

    Ok(Vec::from(key))
}

#[instrument(skip(db), err)]
async fn paste<const N: usize>(
    Extension(db): Extension<Arc<DB>>,
    Path(url): Path<ShortCode<N>>,
) -> Result<Bytes, StatusCode> {
    let key = url.as_bytes();

    let parsed: Paste = {
        // not sure if perf of get_pinned is better than spawn_blocking
        let query_result = db.get_pinned(key).map_err(|e| {
            error!("Failed to fetch initial query: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

        let data = match query_result {
            Some(data) => data,
            None => return Err(StatusCode::NOT_FOUND),
        };

        bincode::deserialize(&data).map_err(|_| {
            error!("Failed to deserialize data?!");
            StatusCode::INTERNAL_SERVER_ERROR
        })?
    };

    if parsed.expired() {
        let join_handle = task::spawn_blocking(move || db.delete(key))
            .await
            .map_err(|e| {
                error!("Failed to join handle: {}", e);
                StatusCode::INTERNAL_SERVER_ERROR
            })?;

        join_handle.map_err(|e| {
            error!("Failed to delete expired paste: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

        return Err(StatusCode::NOT_FOUND);
    }

    if parsed.is_burn_after_read() {
        let join_handle = task::spawn_blocking(move || db.delete(key))
            .await
            .map_err(|e| {
                error!("Failed to join handle: {}", e);
                StatusCode::INTERNAL_SERVER_ERROR
            })?;

        join_handle.map_err(|e| {
            error!("Failed to burn paste after read: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    }

    Ok(parsed.bytes)
}

#[instrument(skip(db))]
async fn delete<const N: usize>(
    Extension(db): Extension<Arc<DB>>,
    Path(url): Path<ShortCode<N>>,
) -> StatusCode {
    match task::spawn_blocking(move || db.delete(url.as_bytes())).await {
        Ok(Ok(_)) => StatusCode::OK,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

/// Periodic clean-up task that deletes expired entries.
async fn cleanup(stop_signal: Arc<AtomicBool>, db: Arc<DB>) {
    while !stop_signal.load(Ordering::Acquire) {
        tokio::time::sleep(*FIVE_MINUTES).await;
        let mut batch = WriteBatch::default();
        for (key, value) in db.snapshot().iterator(IteratorMode::Start) {
            // TODO: only partially decode struct for max perf
            let join_handle = task::spawn_blocking(move || {
                bincode::deserialize::<Paste>(&value)
                    .as_ref()
                    .map(Paste::expired)
                    .unwrap_or_default()
            })
            .await;

            let should_delete = match join_handle {
                Ok(should_delete) => should_delete,
                Err(e) => {
                    error!("Failed to join thread?! {}", e);
                    false
                }
            };

            if should_delete {
                batch.delete(key);
            }
        }

        let db = Arc::clone(&db);
        let join_handle = task::spawn_blocking(move || db.write(batch)).await;
        let db_op_res = match join_handle {
            Ok(res) => res,
            Err(e) => {
                error!("Failed to join handle?! {}", e);
                continue;
            }
        };

        if let Err(e) = db_op_res {
            warn!("Failed to cleanup db: {}", e);
        }
    }
}
