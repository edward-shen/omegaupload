use axum::body::Bytes;
use chrono::{DateTime, Duration, Utc};
use headers::{Header, HeaderName, HeaderValue};
use lazy_static::lazy_static;
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

#[derive(Serialize, Deserialize, Clone, Copy, Debug)]
pub enum Expiration {
    BurnAfterReading,
    UnixTime(DateTime<Utc>),
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
        match values
            .next()
            .ok_or_else(headers::Error::invalid)?
            .as_bytes()
        {
            b"read" => Ok(Self::BurnAfterReading),
            b"5m" => Ok(Self::UnixTime(Utc::now() + Duration::minutes(5))),
            b"10m" => Ok(Self::UnixTime(Utc::now() + Duration::minutes(10))),
            b"1h" => Ok(Self::UnixTime(Utc::now() + Duration::hours(1))),
            b"1d" => Ok(Self::UnixTime(Utc::now() + Duration::days(1))),
            _ => Err(headers::Error::invalid()),
        }
    }

    fn encode<E: Extend<HeaderValue>>(&self, container: &mut E) {
        container.extend(std::iter::once(self.into()));
    }
}

impl From<&Expiration> for HeaderValue {
    fn from(expiration: &Expiration) -> Self {
        unsafe {
            HeaderValue::from_maybe_shared_unchecked(match expiration {
                Expiration::BurnAfterReading => Bytes::from_static(b"0"),
                Expiration::UnixTime(duration) => Bytes::from(duration.to_rfc3339()),
            })
        }
    }
}

impl From<Expiration> for HeaderValue {
    fn from(expiration: Expiration) -> Self {
        (&expiration).into()
    }
}

impl Default for Expiration {
    fn default() -> Self {
        Self::UnixTime(Utc::now() + Duration::days(1))
    }
}
