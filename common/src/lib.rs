#![warn(clippy::nursery, clippy::pedantic)]

//! Contains common functions and structures used by multiple projects

// Copyright (c) 2021 Edward Shen
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in
// all copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
// SOFTWARE.

use std::fmt::Display;
use std::str::FromStr;

use bytes::Bytes;
use chrono::{DateTime, Duration, Utc};
use headers::{Header, HeaderName, HeaderValue};
use lazy_static::lazy_static;
pub use secrecy;
use secrecy::Secret;
use serde::{Deserialize, Serialize};
use thiserror::Error;
pub use url::Url;

use crate::crypto::Key;

pub mod base64;
pub mod crypto;

pub const API_ENDPOINT: &str = "/api";

pub struct ParsedUrl {
    pub sanitized_url: Url,
    pub decryption_key: Secret<Key>,
    pub needs_password: bool,
}

#[derive(Default, Debug)]
pub struct PartialParsedUrl {
    pub decryption_key: Option<Secret<Key>>,
    pub needs_password: bool,
    pub name: Option<String>,
    pub language: Option<String>,
}

#[cfg(test)]
impl PartialEq for PartialParsedUrl {
    fn eq(&self, other: &Self) -> bool {
        use secrecy::ExposeSecret;
        let decryption_key_matches = {
            match (self.decryption_key.as_ref(), other.decryption_key.as_ref()) {
                (Some(key), Some(other)) => key.expose_secret() == other.expose_secret(),
                (None, None) => true,
                _ => false,
            }
        };

        decryption_key_matches && self.needs_password == other.needs_password
    }
}

#[derive(Error, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum PartialParsedUrlParseError {
    #[error("A decryption key that was not valid web base64 was provided.")]
    InvalidDecryptionKey,
}

impl TryFrom<&str> for PartialParsedUrl {
    type Error = PartialParsedUrlParseError;

    fn try_from(fragment: &str) -> Result<Self, Self::Error> {
        // Short circuit if the fragment only contains the key.

        // Base64 has an interesting property that the length of an encoded text
        // is always 4/3rds larger than the original data.
        if !fragment.contains("key:") {
            let decryption_key = base64::decode(fragment)
                .map_err(|_| PartialParsedUrlParseError::InvalidDecryptionKey)?;
            let decryption_key = Key::new_secret(decryption_key);

            return Ok(Self {
                decryption_key,
                ..Self::default()
            });
        }

        let args = fragment.split('!').filter_map(|kv| {
            let (k, v) = {
                let mut iter = kv.split(':');
                (iter.next(), iter.next())
            };

            Some((k?, v))
        });

        let mut decryption_key = None;
        let mut needs_password = false;
        let mut name = None;
        let mut language = None;

        for (key, value) in args {
            match (key, value) {
                ("key", Some(value)) => {
                    let key = base64::decode(value)
                        .map_err(|_| PartialParsedUrlParseError::InvalidDecryptionKey)?;
                    decryption_key = Key::new_secret(key);
                }
                ("pw", _) => {
                    needs_password = true;
                }
                ("name", Some(provided_name)) => name = Some(provided_name.to_owned()),
                ("lang", Some(provided_lang)) => language = Some(provided_lang.to_owned()),
                _ => (),
            }
        }

        Ok(Self {
            decryption_key,
            needs_password,
            name,
            language,
        })
    }
}

impl FromStr for PartialParsedUrl {
    type Err = PartialParsedUrlParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::try_from(s)
    }
}

#[derive(Debug, Error)]
pub enum ParseUrlError {
    #[error("The provided url was bad")]
    BadUrl,
    #[error("Missing decryption key")]
    NeedKey,
    #[error(transparent)]
    InvalidKey(#[from] PartialParsedUrlParseError),
}

impl FromStr for ParsedUrl {
    type Err = ParseUrlError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut url = Url::from_str(s).map_err(|_| ParseUrlError::BadUrl)?;
        let fragment = url.fragment().ok_or(ParseUrlError::NeedKey)?;
        if fragment.is_empty() {
            return Err(ParseUrlError::NeedKey);
        }

        let PartialParsedUrl {
            mut decryption_key,
            needs_password,
            ..
        } = PartialParsedUrl::try_from(fragment)?;

        url.set_fragment(None);

        let decryption_key = decryption_key.take().ok_or(ParseUrlError::NeedKey)?;

        Ok(Self {
            sanitized_url: url,
            decryption_key,
            needs_password,
        })
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug)]
pub enum Expiration {
    BurnAfterReading,
    BurnAfterReadingWithDeadline(DateTime<Utc>),
    UnixTime(DateTime<Utc>),
}

// This impl is used for the CLI. We use a macro here to ensure that possible
// expressed by the CLI are the same supported by the server.
macro_rules! expiration_from_str {
    {
        $($str_repr:literal => $duration:expr),* $(,)?
    } => {
        impl FromStr for Expiration {
            type Err = String;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                match s {
                    $($str_repr => Ok($duration),)*
                    _ => Err(s.to_owned()),
                }
            }
        }

        impl Expiration {
            #[must_use]
            pub const fn variants() -> &'static [&'static str] {
                &[
                    $($str_repr,)*
                ]
            }
        }
    };
}

expiration_from_str! {
    "read" => Self::BurnAfterReading,
    "5m" => Self::UnixTime(Utc::now() + Duration::minutes(5)),
    "10m" => Self::UnixTime(Utc::now() + Duration::minutes(10)),
    "1h" => Self::UnixTime(Utc::now() + Duration::hours(1)),
    "1d" => Self::UnixTime(Utc::now() + Duration::days(1)),
    "3d" => Self::UnixTime(Utc::now() + Duration::days(1)),
    "1w" => Self::UnixTime(Utc::now() + Duration::weeks(1)),
}

impl Display for Expiration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Expiration::BurnAfterReading | Expiration::BurnAfterReadingWithDeadline(_) => {
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
        let bytes = values.next().ok_or_else(headers::Error::invalid)?;

        Self::try_from(bytes).map_err(|_| headers::Error::invalid())
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
                Expiration::BurnAfterReadingWithDeadline(_) | Expiration::BurnAfterReading => {
                    Bytes::from_static(b"0")
                }
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

pub struct ParseHeaderValueError;

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

impl TryFrom<HeaderValue> for Expiration {
    type Error = ParseHeaderValueError;

    fn try_from(value: HeaderValue) -> Result<Self, Self::Error> {
        Self::try_from(&value)
    }
}

impl TryFrom<&HeaderValue> for Expiration {
    type Error = ParseHeaderValueError;

    fn try_from(value: &HeaderValue) -> Result<Self, Self::Error> {
        std::str::from_utf8(value.as_bytes())
            .map_err(|_| ParseHeaderValueError)
            .and_then(Self::try_from)
    }
}

impl TryFrom<&str> for Expiration {
    type Error = ParseHeaderValueError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        if value == "0" {
            return Ok(Self::BurnAfterReading);
        }

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

#[cfg(test)]
mod partial_parsed_url_parsing {
    use secrecy::Secret;

    use crate::base64;
    use crate::crypto::Key;
    use crate::PartialParsedUrl;

    #[test]
    fn empty() {
        assert_eq!("".parse(), Ok(PartialParsedUrl::default()));
    }

    const DECRYPTION_KEY_STRING: &str = "ddLod7sGy_EjFDjWqZoH4i5n_XU8bIpEuEo3-pjfAIE=";

    fn decryption_key() -> Option<Secret<Key>> {
        Key::new_secret(base64::decode(DECRYPTION_KEY_STRING).unwrap())
    }

    #[test]
    fn clean_no_password() {
        assert_eq!(
            DECRYPTION_KEY_STRING.parse(),
            Ok(PartialParsedUrl {
                decryption_key: decryption_key(),
                ..Default::default()
            })
        );
    }

    #[test]
    fn no_password() {
        let input = "key:ddLod7sGy_EjFDjWqZoH4i5n_XU8bIpEuEo3-pjfAIE=";
        assert_eq!(
            input.parse(),
            Ok(PartialParsedUrl {
                decryption_key: decryption_key(),
                ..Default::default()
            })
        );
    }

    #[test]
    fn with_password() {
        let input = "key:ddLod7sGy_EjFDjWqZoH4i5n_XU8bIpEuEo3-pjfAIE=!pw";
        assert_eq!(
            input.parse(),
            Ok(PartialParsedUrl {
                decryption_key: decryption_key(),
                needs_password: true,
                ..Default::default()
            })
        );
    }

    #[test]
    fn with_name() {
        let input = "key:ddLod7sGy_EjFDjWqZoH4i5n_XU8bIpEuEo3-pjfAIE=!name:test_file.rs";
        assert_eq!(
            input.parse(),
            Ok(PartialParsedUrl {
                decryption_key: decryption_key(),
                name: Some("test_file.rs".to_owned()),
                ..Default::default()
            })
        );
    }

    #[test]
    fn with_lang() {
        let input = "key:ddLod7sGy_EjFDjWqZoH4i5n_XU8bIpEuEo3-pjfAIE=!lang:rust";
        assert_eq!(
            input.parse(),
            Ok(PartialParsedUrl {
                decryption_key: decryption_key(),
                language: Some("rust".to_owned()),
                ..Default::default()
            })
        );
    }

    #[test]
    fn order_does_not_matter() {
        let input = "pw!key:ddLod7sGy_EjFDjWqZoH4i5n_XU8bIpEuEo3-pjfAIE=";
        assert_eq!(
            input.parse(),
            Ok(PartialParsedUrl {
                decryption_key: decryption_key(),
                needs_password: true,
                ..Default::default()
            })
        );
    }

    #[test]
    fn empty_key_pair_gracefully_fails() {
        let input = "!!!key:ddLod7sGy_EjFDjWqZoH4i5n_XU8bIpEuEo3-pjfAIE=!!!";
        assert_eq!(
            input.parse(),
            Ok(PartialParsedUrl {
                decryption_key: decryption_key(),
                ..Default::default()
            })
        );
    }

    #[test]
    fn invalid_decryption_key_fails() {
        assert!("invalid key".parse::<PartialParsedUrl>().is_err());
    }

    #[test]
    fn unknown_fields_fail() {
        assert!("!!a!!b!!c".parse::<PartialParsedUrl>().is_err());
    }
}
