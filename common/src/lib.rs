#![warn(clippy::nursery, clippy::pedantic)]

//! Contains common functions and structures used by multiple projects

use std::fmt::Display;
use std::str::FromStr;

use bytes::Bytes;
use chrono::{DateTime, Duration, Utc};
use headers::{Header, HeaderName, HeaderValue};
use lazy_static::lazy_static;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
pub use url::Url;

use crate::crypto::{Key, Nonce};

pub const API_ENDPOINT: &str = "/api";

pub mod base64 {
    /// URL-safe Base64 encoding.
    pub fn encode(input: impl AsRef<[u8]>) -> String {
        base64::encode_config(input, base64::URL_SAFE)
    }

    /// URL-safe Base64 decoding.
    pub fn decode(input: impl AsRef<[u8]>) -> Result<Vec<u8>, base64::DecodeError> {
        base64::decode_config(input, base64::URL_SAFE)
    }
}

/// Hashes an input to output a usable key.
pub fn hash(data: impl AsRef<[u8]>) -> crypto::Key {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher.finalize()
}

pub mod crypto {
    use std::ops::{Deref, DerefMut};

    use chacha20poly1305::aead::generic_array::GenericArray;
    use chacha20poly1305::aead::{Aead, AeadInPlace, Buffer, Error, NewAead};
    use chacha20poly1305::XChaCha20Poly1305;
    use chacha20poly1305::XNonce;
    use rand::{thread_rng, Rng};

    pub use chacha20poly1305::Key;

    /// Securely generates a random key and nonce.
    #[must_use]
    pub fn gen_key_nonce() -> (Key, Nonce) {
        let mut rng = thread_rng();
        let mut key: Key = GenericArray::default();
        rng.fill(key.as_mut_slice());
        let mut nonce = Nonce::default();
        rng.fill(nonce.as_mut_slice());
        (key, nonce)
    }

    pub fn seal(plaintext: &[u8], nonce: &Nonce, key: &Key) -> Result<Vec<u8>, Error> {
        let cipher = XChaCha20Poly1305::new(key);
        cipher.encrypt(nonce, plaintext)
    }

    pub fn seal_in_place(buffer: &mut impl Buffer, nonce: &Nonce, key: &Key) -> Result<(), Error> {
        let cipher = XChaCha20Poly1305::new(key);
        cipher.encrypt_in_place(nonce, &[], buffer)
    }

    pub fn open(encrypted: &[u8], nonce: &Nonce, key: &Key) -> Result<Vec<u8>, Error> {
        let cipher = XChaCha20Poly1305::new(key);
        cipher.decrypt(nonce, encrypted)
    }

    pub fn open_in_place(buffer: &mut impl Buffer, nonce: &Nonce, key: &Key) -> Result<(), Error> {
        let cipher = XChaCha20Poly1305::new(key);
        cipher.decrypt_in_place(nonce, &[], buffer)
    }

    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    pub struct Nonce(XNonce);

    impl Default for Nonce {
        fn default() -> Self {
            Self(GenericArray::default())
        }
    }

    impl Deref for Nonce {
        type Target = XNonce;

        fn deref(&self) -> &Self::Target {
            &self.0
        }
    }

    impl DerefMut for Nonce {
        fn deref_mut(&mut self) -> &mut Self::Target {
            &mut self.0
        }
    }

    impl AsRef<[u8]> for Nonce {
        fn as_ref(&self) -> &[u8] {
            self.0.as_ref()
        }
    }

    impl Nonce {
        #[must_use]
        pub fn increment(&self) -> Self {
            let mut inner = self.0;
            inner.as_mut_slice()[0] += 1;
            Self(inner)
        }

        #[must_use]
        pub fn from_slice(slice: &[u8]) -> Self {
            Self(*XNonce::from_slice(slice))
        }
    }
}

pub struct ParsedUrl {
    pub sanitized_url: Url,
    pub decryption_key: Key,
    pub nonce: Nonce,
    pub needs_password: bool,
}

#[derive(Default)]
pub struct PartialParsedUrl {
    pub decryption_key: Option<Key>,
    pub nonce: Option<Nonce>,
    pub needs_password: bool,
}

impl From<&str> for PartialParsedUrl {
    fn from(fragment: &str) -> Self {
        let args = fragment.split('!').filter_map(|kv| {
            let (k, v) = {
                let mut iter = kv.split(':');
                (iter.next(), iter.next())
            };

            Some((k?, v))
        });

        let mut decryption_key = None;
        let mut needs_password = false;
        let mut nonce = None;

        for (key, value) in args {
            match (key, value) {
                ("key", Some(value)) => {
                    decryption_key = base64::decode(value).map(|k| *Key::from_slice(&k)).ok();
                }
                ("pw", _) => {
                    needs_password = true;
                }
                ("nonce", Some(value)) => {
                    nonce = base64::decode(value).as_deref().map(Nonce::from_slice).ok();
                }
                _ => (),
            }
        }

        Self {
            decryption_key,
            nonce,
            needs_password,
        }
    }
}

#[derive(Debug, Error)]
pub enum ParseUrlError {
    #[error("The provided url was bad")]
    BadUrl,
    #[error("Missing decryption key")]
    NeedKey,
    #[error("Missing nonce")]
    NeedNonce,
    #[error("Missing decryption key and nonce")]
    NeedKeyAndNonce,
}

impl FromStr for ParsedUrl {
    type Err = ParseUrlError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut url = Url::from_str(s).map_err(|_| ParseUrlError::BadUrl)?;
        let fragment = url.fragment().ok_or(ParseUrlError::NeedKeyAndNonce)?;
        if fragment.is_empty() {
            return Err(ParseUrlError::NeedKeyAndNonce);
        }

        let PartialParsedUrl {
            decryption_key,
            needs_password,
            nonce,
        } = PartialParsedUrl::from(fragment);

        url.set_fragment(None);

        let (decryption_key, nonce) = match (&decryption_key, nonce) {
            (None, None) => Err(ParseUrlError::NeedKeyAndNonce),
            (None, Some(_)) => Err(ParseUrlError::NeedKey),
            (Some(_), None) => Err(ParseUrlError::NeedNonce),
            (Some(k), Some(v)) => Ok((*k, v)),
        }?;

        Ok(Self {
            sanitized_url: url,
            decryption_key,
            needs_password,
            nonce,
        })
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug)]
pub enum Expiration {
    BurnAfterReading,
    UnixTime(DateTime<Utc>),
}

impl Display for Expiration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Expiration::BurnAfterReading => {
                write!(f, "This item has been burned. You now have the only copy.")
            }
            Expiration::UnixTime(time) => write!(
                f,
                "{}",
                time.format("This item will expire on %A, %B %-d, %Y at %T %Z.")
            ),
        }
    }
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
            // We disallow permanent pastes
            _ => Err(headers::Error::invalid()),
        }
    }

    fn encode<E: Extend<HeaderValue>>(&self, container: &mut E) {
        container.extend(std::iter::once(self.into()));
    }
}

impl From<&Expiration> for HeaderValue {
    fn from(expiration: &Expiration) -> Self {
        // SAFETY: All possible values of `Expiration` are valid header values,
        // so we don't need the extra check.
        unsafe {
            Self::from_maybe_shared_unchecked(match expiration {
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

#[cfg(feature = "wasm")]
impl TryFrom<web_sys::Headers> for Expiration {
    type Error = ParseHeaderValueError;

    fn try_from(headers: web_sys::Headers) -> Result<Self, Self::Error> {
        headers
            .get(http::header::EXPIRES.as_str())
            .ok()
            .flatten()
            .as_deref()
            .and_then(|v| Self::try_from(v).ok())
            .ok_or(ParseHeaderValueError)
    }
}

pub struct ParseHeaderValueError;

impl TryFrom<&HeaderValue> for Expiration {
    type Error = ParseHeaderValueError;

    fn try_from(value: &HeaderValue) -> Result<Self, Self::Error> {
        value
            .to_str()
            .map_err(|_| ParseHeaderValueError)
            .and_then(Self::try_from)
    }
}

impl TryFrom<&str> for Expiration {
    type Error = ParseHeaderValueError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        value
            .parse::<DateTime<Utc>>()
            .map_err(|_| ParseHeaderValueError)
            .map(Self::UnixTime)
    }
}

impl Default for Expiration {
    fn default() -> Self {
        Self::UnixTime(Utc::now() + Duration::days(1))
    }
}
