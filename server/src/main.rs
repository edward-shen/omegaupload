#![warn(clippy::nursery, clippy::pedantic)]

// OmegaUpload Zero Knowledge File Hosting
// Copyright (C) 2021  Edward Shen
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use axum::body::Bytes;
use axum::error_handling::HandleError;
use axum::extract::{Extension, Path, TypedHeader};
use axum::http::header::EXPIRES;
use axum::http::StatusCode;
use axum::response::Html;
use axum::routing::{get, get_service, post};
use axum::{AddExtensionLayer, Router};
use chrono::Utc;
use futures::stream::StreamExt;
use headers::HeaderMap;
use lazy_static::lazy_static;
use omegaupload_common::crypto::get_csrng;
use omegaupload_common::{Expiration, API_ENDPOINT};
use rand::Rng;
use rocksdb::{ColumnFamilyDescriptor, IteratorMode};
use rocksdb::{Options, DB};
use signal_hook::consts::SIGUSR1;
use signal_hook_tokio::Signals;
use tokio::task::{self, JoinHandle};
use tower_http::services::ServeDir;
use tracing::{error, instrument, trace};
use tracing::{info, warn};

use crate::short_code::ShortCode;

mod short_code;

const BLOB_CF_NAME: &str = "blob";
const META_CF_NAME: &str = "meta";

lazy_static! {
    static ref MAX_PASTE_AGE: chrono::Duration = chrono::Duration::days(1);
}

#[tokio::main]
async fn main() -> Result<()> {
    const INDEX_PAGE: Html<&'static str> = Html(include_str!("../../dist/index.html"));
    const PASTE_DB_PATH: &str = "database";
    const SHORT_CODE_SIZE: usize = 12;

    tracing_subscriber::fmt::init();

    let mut db_options = Options::default();
    db_options.create_if_missing(true);
    db_options.create_missing_column_families(true);
    db_options.set_compression_type(rocksdb::DBCompressionType::Zstd);
    let db = Arc::new(DB::open_cf_descriptors(
        &db_options,
        PASTE_DB_PATH,
        [
            ColumnFamilyDescriptor::new(BLOB_CF_NAME, Options::default()),
            ColumnFamilyDescriptor::new(META_CF_NAME, Options::default()),
        ],
    )?);

    set_up_expirations::<SHORT_CODE_SIZE>(&db);

    let signals = Signals::new(&[SIGUSR1])?;
    let signals_handle = signals.handle();
    let signals_task = tokio::spawn(handle_signals(signals, Arc::clone(&db)));

    let root_service = HandleError::new(get_service(ServeDir::new("static")), |_| async {
        Ok::<_, Infallible>(StatusCode::NOT_FOUND)
    });

    axum::Server::bind(&"0.0.0.0:8080".parse()?)
        .serve({
            info!("Now serving on 0.0.0.0:8080");
            Router::new()
                .route(
                    "/",
                    post(upload::<SHORT_CODE_SIZE>).get(|| async { INDEX_PAGE }),
                )
                .route("/:code", get(|| async { INDEX_PAGE }))
                .nest("/static", root_service)
                .route(
                    &format!("{API_ENDPOINT}/:code"),
                    get(paste::<SHORT_CODE_SIZE>).delete(delete::<SHORT_CODE_SIZE>),
                )
                .layer(AddExtensionLayer::new(db))
                .into_make_service()
        })
        .await?;

    // Must be called for correct shutdown
    DB::destroy(&Options::default(), PASTE_DB_PATH)?;

    signals_handle.close();
    signals_task.await?;
    Ok(())
}

// See https://link.eddie.sh/5JHlD
#[allow(clippy::cognitive_complexity)]
fn set_up_expirations<const N: usize>(db: &Arc<DB>) {
    let mut corrupted = 0;
    let mut expired = 0;
    let mut pending = 0;

    info!("Setting up cleanup timers, please wait...");

    let meta_cf = db.cf_handle(META_CF_NAME).unwrap();

    let db_ref = Arc::clone(db);

    for (key, value) in db.iterator_cf(meta_cf, IteratorMode::Start) {
        let key: [u8; N] = (*key).try_into().unwrap();

        let expiration = if let Ok(value) = bincode::deserialize::<Expiration>(&value) {
            value
        } else {
            corrupted += 1;
            delete_entry(Arc::clone(&db_ref), key);
            continue;
        };

        let expiration_time = match expiration {
            Expiration::BurnAfterReading => {
                warn!("Found unbounded burn after reading. Defaulting to max age");
                Utc::now() + *MAX_PASTE_AGE
            }
            Expiration::BurnAfterReadingWithDeadline(deadline) => deadline,
            Expiration::UnixTime(time) => time,
        };

        let sleep_duration = (expiration_time - Utc::now()).to_std().unwrap_or_default();
        if sleep_duration == Duration::default() {
            expired += 1;
            delete_entry(Arc::clone(&db_ref), key);
        } else {
            pending += 1;
            let db = Arc::clone(&db_ref);
            task::spawn(async move {
                tokio::time::sleep(sleep_duration).await;
                delete_entry(db, key);
            });
        }
    }

    if corrupted == 0 {
        info!("No corrupted pastes found.");
    } else {
        warn!("Found {corrupted} corrupted pastes.");
    }

    info!("Found {expired} expired pastes.");
    info!("Found {pending} active pastes.");
    info!("Cleanup timers have been initialized.");
}

async fn handle_signals(mut signals: Signals, db: Arc<DB>) {
    while let Some(signal) = signals.next().await {
        if signal == SIGUSR1 {
            let meta_cf = db.cf_handle(META_CF_NAME).unwrap();
            info!(
                "Active paste count: {}",
                db.iterator_cf(meta_cf, IteratorMode::Start).count()
            );
        }
    }
}

#[instrument(skip(db, body), err)]
async fn upload<const N: usize>(
    Extension(db): Extension<Arc<DB>>,
    maybe_expires: Option<TypedHeader<Expiration>>,
    body: Bytes,
) -> Result<Vec<u8>, StatusCode> {
    if body.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }

    if let Some(header) = maybe_expires {
        if let Expiration::UnixTime(time) = header.0 {
            if (time - Utc::now()) > *MAX_PASTE_AGE {
                warn!("{time} exceeds allowed paste lifetime");
                return Err(StatusCode::BAD_REQUEST);
            }
        }
    }

    // 3GB max; this is a soft-limit of RocksDb
    if body.len() >= 3_221_225_472 {
        return Err(StatusCode::PAYLOAD_TOO_LARGE);
    }

    let mut new_key = None;

    trace!("Generating short code...");

    // Try finding a code; give up after 1000 attempts
    // Statistics show that this is very unlikely to happen
    for i in 0..1000 {
        let code: ShortCode<N> = get_csrng().sample(short_code::Generator);
        let db = Arc::clone(&db);
        let key = code.as_bytes();
        let query = task::spawn_blocking(move || {
            db.key_may_exist_cf(db.cf_handle(META_CF_NAME).unwrap(), key)
        })
        .await;
        if matches!(query, Ok(false)) {
            new_key = Some(key);
            trace!("Found new key after {i} attempts.");
            break;
        }
    }

    let key = if let Some(key) = new_key {
        key
    } else {
        error!("Failed to generate a valid short code!");
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    };

    let db_ref = Arc::clone(&db);
    match task::spawn_blocking(move || {
        let blob_cf = db_ref.cf_handle(BLOB_CF_NAME).unwrap();
        let meta_cf = db_ref.cf_handle(META_CF_NAME).unwrap();
        let data = bincode::serialize(&body).expect("bincode to serialize");
        db_ref.put_cf(blob_cf, key, data)?;
        let expires = maybe_expires.map(|v| v.0).unwrap_or_default();
        let expires = if let Expiration::BurnAfterReading = expires {
            Expiration::BurnAfterReadingWithDeadline(Utc::now() + *MAX_PASTE_AGE)
        } else {
            expires
        };
        let meta = bincode::serialize(&expires).expect("bincode to serialize");
        if db_ref.put_cf(meta_cf, key, meta).is_err() {
            // try and roll back on metadata write failure
            db_ref.delete_cf(blob_cf, key)?;
        }
        Result::<_, anyhow::Error>::Ok(())
    })
    .await
    {
        Ok(Ok(_)) => {
            if let Some(expires) = maybe_expires {
                if let Expiration::UnixTime(expiration_time)
                | Expiration::BurnAfterReadingWithDeadline(expiration_time) = expires.0
                {
                    let sleep_duration =
                        (expiration_time - Utc::now()).to_std().unwrap_or_default();
                    task::spawn(async move {
                        tokio::time::sleep(sleep_duration).await;
                        delete_entry(db, key);
                    });
                }
            }
        }
        e => {
            error!("Failed to insert paste into db: {e:?}");
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

    let metadata: Expiration = {
        let meta_cf = db.cf_handle(META_CF_NAME).unwrap();
        let query_result = db.get_cf(meta_cf, key).map_err(|e| {
            error!("Failed to fetch initial query: {e}");
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

    // Check if paste has expired.
    if let Expiration::UnixTime(expires) = metadata {
        if expires < Utc::now() {
            delete_entry(db, url.as_bytes()).await.map_err(|e| {
                error!("Failed to join handle: {e}");
                StatusCode::INTERNAL_SERVER_ERROR
            })??;
            return Err(StatusCode::NOT_FOUND);
        }
    }

    let paste: Bytes = {
        // not sure if perf of get_pinned is better than spawn_blocking
        let blob_cf = db.cf_handle(BLOB_CF_NAME).unwrap();
        let query_result = db.get_pinned_cf(blob_cf, key).map_err(|e| {
            error!("Failed to fetch initial query: {e}");
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

    // Check if we need to burn after read
    if matches!(
        metadata,
        Expiration::BurnAfterReading | Expiration::BurnAfterReadingWithDeadline(_)
    ) {
        delete_entry(db, key).await.map_err(|e| {
            error!("Failed to join handle: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        })??;
    }

    let mut map = HeaderMap::new();
    map.insert(EXPIRES, metadata.into());

    Ok((map, paste))
}

#[instrument(skip(db))]
async fn delete<const N: usize>(
    Extension(db): Extension<Arc<DB>>,
    Path(url): Path<ShortCode<N>>,
) -> StatusCode {
    match delete_entry(db, url.as_bytes()).await {
        Ok(_) => StatusCode::OK,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

fn delete_entry<const N: usize>(db: Arc<DB>, key: [u8; N]) -> JoinHandle<Result<(), StatusCode>> {
    task::spawn_blocking(move || {
        let blob_cf = db.cf_handle(BLOB_CF_NAME).unwrap();
        let meta_cf = db.cf_handle(META_CF_NAME).unwrap();
        if let Err(e) = db.delete_cf(blob_cf, &key) {
            warn!("{e}");
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
        if let Err(e) = db.delete_cf(meta_cf, &key) {
            warn!("{e}");
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
        Ok(())
    })
}
