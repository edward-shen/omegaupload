use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::body::Bytes;
use headers::{Header, HeaderName, HeaderValue};
use lazy_static::lazy_static;
use serde::{Deserialize, Serialize};

use crate::time::{FIVE_MINUTES, ONE_DAY, ONE_HOUR, TEN_MINUTES};

#[derive(Serialize, Deserialize)]
pub struct Paste {
    expiration: Option<Expiration>,
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
                Expiration::UnixTime(expiration) => {
                    let now = time_since_unix();
                    expiration < now
                }
            })
            .unwrap_or_default()
    }

    pub const fn is_burn_after_read(&self) -> bool {
        matches!(self.expiration, Some(Expiration::BurnAfterReading))
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug)]
pub enum Expiration {
    BurnAfterReading,
    UnixTime(Duration),
}

lazy_static! {
    pub static ref EXPIRATION_HEADER_NAME: HeaderName = HeaderName::from_static("burn-after");
}

impl Header for Expiration {
    fn name() -> &'static HeaderName {
        &*EXPIRATION_HEADER_NAME
    }

    fn decode<'i, I>(values: &mut I) -> Result<Self, headers::Error>
    where
        Self: Sized,
        I: Iterator<Item = &'i HeaderValue>,
    {
        let now = time_since_unix();
        match values
            .next()
            .ok_or_else(headers::Error::invalid)?
            .as_bytes()
        {
            b"read" => Ok(Self::BurnAfterReading),
            b"5m" => Ok(Self::UnixTime(now + *FIVE_MINUTES)),
            b"10m" => Ok(Self::UnixTime(now + *TEN_MINUTES)),
            b"1h" => Ok(Self::UnixTime(now + *ONE_HOUR)),
            b"1d" => Ok(Self::UnixTime(now + *ONE_DAY)),
            _ => Err(headers::Error::invalid()),
        }
    }

    fn encode<E: Extend<HeaderValue>>(&self, _: &mut E) {
        unimplemented!("This shouldn't need implementation")
    }
}

impl Default for Expiration {
    fn default() -> Self {
        Self::UnixTime(time_since_unix() + *ONE_DAY)
    }
}

fn time_since_unix() -> Duration {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time since epoch to always work")
}
