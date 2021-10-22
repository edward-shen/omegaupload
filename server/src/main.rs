#![warn(clippy::nursery, clippy::pedantic)]

use std::sync::Arc;

use anyhow::Result;
use axum::body::Bytes;
use axum::extract::{Extension, Path, TypedHeader};
use axum::handler::{get, post};
use axum::http::header::EXPIRES;
use axum::http::StatusCode;
use axum::{AddExtensionLayer, Router};
use chrono::Utc;
use headers::HeaderMap;
use omegaupload_common::Expiration;
use rand::thread_rng;
use rand::Rng;
use rocksdb::IteratorMode;
use rocksdb::{Options, DB};
use tokio::task;
use tracing::{error, instrument};
use tracing::{info, warn};

use crate::paste::Paste;
use crate::short_code::ShortCode;

mod paste;
mod short_code;

#[tokio::main]
async fn main() -> Result<()> {
    const DB_PATH: &str = "database";
    const SHORT_CODE_SIZE: usize = 12;

    tracing_subscriber::fmt::init();

    let db = Arc::new(DB::open_default(DB_PATH)?);

    set_up_expirations(Arc::clone(&db));

    axum::Server::bind(&"0.0.0.0:8081".parse()?)
        .serve(
            Router::new()
                .route("/", post(upload::<SHORT_CODE_SIZE>))
                .route(
                    "/:code",
                    get(paste::<SHORT_CODE_SIZE>).delete(delete::<SHORT_CODE_SIZE>),
                )
                .layer(AddExtensionLayer::new(db))
                .into_make_service(),
        )
        .await?;

    // Must be called for correct shutdown
    DB::destroy(&Options::default(), DB_PATH)?;
    Ok(())
}

fn set_up_expirations(db: Arc<DB>) {
    let mut corrupted = 0;
    let mut expired = 0;
    let mut pending = 0;
    let mut permanent = 0;

    info!("Setting up cleanup timers, please wait...");

    for (key, value) in db.iterator(IteratorMode::Start) {
        let paste = if let Ok(value) = bincode::deserialize::<Paste>(&value) {
            value
        } else {
            corrupted += 1;
            if let Err(e) = db.delete(key) {
                warn!("{}", e);
            }
            continue;
        };
        if let Some(Expiration::UnixTime(time)) = paste.expiration {
            let now = Utc::now();

            if time < now {
                expired += 1;
                if let Err(e) = db.delete(key) {
                    warn!("{}", e);
                }
            } else {
                let sleep_duration = (time - now).to_std().unwrap();
                pending += 1;

                let db_ref = Arc::clone(&db);
                task::spawn_blocking(move || async move {
                    tokio::time::sleep(sleep_duration).await;
                    if let Err(e) = db_ref.delete(key) {
                        warn!("{}", e);
                    }
                });
            }
        } else {
            permanent += 1;
        }
    }

    if corrupted == 0 {
        info!("No corrupted pastes found.");
    } else {
        warn!("Found {} corrupted pastes.", corrupted);
    }
    info!("Found {} expired pastes.", expired);
    info!("Found {} active pastes.", pending);
    info!("Found {} permanent pastes.", permanent);
    info!("Cleanup timers have been initialized.");
}

#[instrument(skip(db), err)]
async fn upload<const N: usize>(
    Extension(db): Extension<Arc<DB>>,
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
        let code: ShortCode<N> = thread_rng().sample(short_code::Generator);
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

    let db_ref = Arc::clone(&db);
    match task::spawn_blocking(move || db_ref.put(key, value)).await {
        Ok(Ok(_)) => {
            if let Some(expires) = maybe_expires {
                if let Expiration::UnixTime(time) = expires.0 {
                    let now = Utc::now();

                    if time < now {
                        if let Err(e) = db.delete(key) {
                            warn!("{}", e);
                        }
                    } else {
                        let sleep_duration = (time - now).to_std().unwrap();

                        task::spawn_blocking(move || async move {
                            tokio::time::sleep(sleep_duration).await;
                            if let Err(e) = db.delete(key) {
                                warn!("{}", e);
                            }
                        });
                    }
                }
            }
        }
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
) -> Result<(HeaderMap, Bytes), StatusCode> {
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

    let mut map = HeaderMap::new();
    if let Some(expiration) = parsed.expiration {
        map.insert(EXPIRES, expiration.into());
    }
    Ok((map, parsed.bytes))
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
