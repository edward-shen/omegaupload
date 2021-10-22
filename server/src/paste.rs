use axum::body::Bytes;
use chrono::Utc;
use omegaupload_common::Expiration;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct Paste {
    pub expiration: Option<Expiration>,
    pub bytes: Bytes,
}

impl Paste {
    pub fn new(expiration: impl Into<Option<Expiration>>, bytes: Bytes) -> Self {
        Self {
            expiration: expiration.into(),
            bytes,
        }
    }

    pub fn expired(&self) -> bool {
        self.expiration
            .map(|expires| match expires {
                Expiration::BurnAfterReading => false,
                Expiration::UnixTime(expiration) => expiration < Utc::now(),
            })
            .unwrap_or_default()
    }

    pub const fn is_burn_after_read(&self) -> bool {
        matches!(self.expiration, Some(Expiration::BurnAfterReading))
    }
}
