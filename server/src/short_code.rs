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

use std::fmt::Debug;

use rand::prelude::Distribution;
use rand::Rng;
use serde::de::{Unexpected, Visitor};
use serde::Deserialize;

pub struct ShortCode<const N: usize>([ShortCodeChar; N]);

impl<const N: usize> ShortCode<N> {
    pub fn as_bytes(&self) -> [u8; N] {
        self.0.map(|v| v.0 as u8)
    }
}

impl<const N: usize> Debug for ShortCode<N> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let short_code = String::from_iter(self.0.map(|v| v.0));
        f.debug_tuple("ShortCode").field(&short_code).finish()
    }
}

impl<'de, const N: usize> Deserialize<'de> for ShortCode<N> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct ShortCodeVisitor<const N: usize>;
        impl<'de, const N: usize> Visitor<'de> for ShortCodeVisitor<N> {
            type Value = ShortCode<N>;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a valid shortcode")
            }

            fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                if v.len() != N {
                    return Err(E::invalid_length(v.len(), &"a 12 character value"));
                }

                if !v.is_ascii() {
                    return Err(E::invalid_value(Unexpected::Str(v), &"ascii only"));
                }

                // This is fine, it'll get overwritten anyways.
                let mut output = [ShortCodeChar('\0'); N];
                for (i, c) in v.char_indices() {
                    output[i] = c.try_into().map_err(|_| {
                        E::invalid_value(Unexpected::Char(c), &"a valid short code character")
                    })?;
                }

                Ok(ShortCode(output))
            }
        }

        deserializer.deserialize_str(ShortCodeVisitor)
    }
}

/// `ShortCodeChar` uses the Word-safe alphabet, a Base32 extension of the Open
/// Location Code Base20 alphabet.
#[derive(Clone, Copy, Debug)]
struct ShortCodeChar(char);

impl<'de> Deserialize<'de> for ShortCodeChar {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct ShortCodeCharVisitor;
        impl<'de> Visitor<'de> for ShortCodeCharVisitor {
            type Value = ShortCodeChar;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a valid short code char")
            }

            fn visit_char<E>(self, v: char) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                v.try_into().map_err(|_| {
                    E::invalid_value(Unexpected::Char(v as char), &"a valid short code character")
                })
            }
        }

        deserializer.deserialize_char(ShortCodeCharVisitor)
    }
}

impl TryFrom<char> for ShortCodeChar {
    type Error = &'static str;

    fn try_from(v: char) -> Result<Self, Self::Error> {
        if v.is_ascii() && ALPHABET.contains(&(v as u8)) {
            Ok(Self(v))
        } else {
            Err("a valid short code character")
        }
    }
}

pub struct Generator;

const ALPHABET: &[u8; 32] = b"23456789CFGHJMPQRVWXcfghjmpqrvwx";

impl Distribution<ShortCodeChar> for Generator {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> ShortCodeChar {
        let value = rng.gen_range(0..32);
        assert!(value < 32);
        ShortCodeChar(ALPHABET[value] as char)
    }
}

impl<const N: usize> Distribution<ShortCode<N>> for Generator {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> ShortCode<N> {
        let mut arr = [ShortCodeChar('\0'); N];

        for c in arr.iter_mut() {
            *c = self.sample(rng);
        }

        ShortCode(arr)
    }
}
