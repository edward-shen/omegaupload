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

use std::ops::{Deref, DerefMut};

use argon2::{Argon2, ParamsBuilder};
use chacha20poly1305::aead::generic_array::sequence::GenericSequence;
use chacha20poly1305::aead::generic_array::GenericArray;
use chacha20poly1305::aead::{AeadInPlace, NewAead};
use chacha20poly1305::XChaCha20Poly1305;
use chacha20poly1305::XNonce;
use rand::{CryptoRng, Rng};
use secrecy::{DebugSecret, ExposeSecret, Secret, SecretVec, Zeroize};
use typenum::Unsigned;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Invalid password.")]
    Password,
    #[error("Invalid secret key.")]
    SecretKey,
    #[error("An error occurred while trying to decrypt the blob.")]
    Encryption,
    #[error("An error occurred while trying to derive a secret key.")]
    Kdf,
}

// This struct intentionally prevents implement Clone or Copy
#[derive(Default, PartialEq, Eq)]
pub struct Key(chacha20poly1305::Key);

impl Key {
    /// Encloses a secret key in a secret `Key` struct.
    pub fn new_secret(vec: Vec<u8>) -> Option<Secret<Self>> {
        chacha20poly1305::Key::from_exact_iter(vec.into_iter())
            .map(Self)
            .map(Secret::new)
    }
}

impl DebugSecret for Key {}

impl AsRef<chacha20poly1305::Key> for Key {
    fn as_ref(&self) -> &chacha20poly1305::Key {
        &self.0
    }
}

impl Deref for Key {
    type Target = chacha20poly1305::Key;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for Key {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Zeroize for Key {
    fn zeroize(&mut self) {
        self.0.zeroize();
    }
}

/// Seals the provided message with an optional password, returning the secret
/// key used to encrypt the message and mutating the buffer to contain necessary
/// metadata.
///
/// The resulting sealed message has the nonce used to encrypt the message
/// appended to it as well as a salt string used to derive the key. In other
/// words, the modified buffer is one of the following to possibilities,
/// depending if there was a password provided:
///
/// ```text
/// modified = C(message, rng_key, nonce) || nonce
/// ```
/// or
/// ```text
/// modified = C(C(message, rng_key, nonce), kdf(pw, salt), nonce + 1) || nonce || salt
/// ```
///
/// Where:
///  - `C(message, key, nonce)` represents encrypting a provided message with
///     `XChaCha20Poly1305`.
///  - `rng_key` represents a randomly generated key.
///  - `kdf(pw, salt)` represents a key derived from Argon2.
///  - `nonce` represents a randomly generated nonce.
///
/// Note that the lengths for the nonce, key, and salt follow recommended
/// values. As of writing this doc (2021-10-31), the nonce size is 24 bytes, the
/// salt size is 16 bytes, and the key size is 32 bytes.
///
/// # Errors
///
/// This message will return an error if and only if there was a problem
/// encrypting the message or deriving a secret key from the password, if one
/// was provided.
pub fn seal_in_place(
    message: &mut Vec<u8>,
    pw: Option<SecretVec<u8>>,
) -> Result<Secret<Key>, Error> {
    let (key, nonce) = gen_key_nonce();
    let cipher = XChaCha20Poly1305::new(key.expose_secret());
    cipher
        .encrypt_in_place(&nonce, &[], message)
        .map_err(|_| Error::Encryption)?;

    let mut maybe_salt_string = None;
    if let Some(password) = pw {
        let (key, salt_string) = kdf(&password).map_err(|_| Error::Kdf)?;
        maybe_salt_string = Some(salt_string);
        let cipher = XChaCha20Poly1305::new(key.expose_secret());
        cipher
            .encrypt_in_place(&nonce.increment(), &[], message)
            .map_err(|_| Error::Encryption)?;
    }

    message.extend_from_slice(nonce.as_slice());
    if let Some(maybe_salted_string) = maybe_salt_string {
        message.extend_from_slice(maybe_salted_string.as_ref());
    }
    Ok(key)
}

/// Opens a message that has been sealed with `seal_in_place`.
///
/// # Errors
///
/// Returns an error if there was a decryption failure or if there was a problem
/// deriving a secret key from the password.
pub fn open_in_place(
    data: &mut Vec<u8>,
    key: &Secret<Key>,
    password: Option<SecretVec<u8>>,
) -> Result<(), Error> {
    let pw_key = if let Some(password) = password {
        let salt_buf = data.split_off(data.len() - Salt::SIZE);
        let argon = get_argon2();
        let mut pw_key = Key::default();
        argon
            .hash_password_into(password.expose_secret(), &salt_buf, &mut pw_key)
            .map_err(|_| Error::Kdf)?;
        Some(Secret::new(pw_key))
    } else {
        None
    };

    let nonce = Nonce::from_slice(&data.split_off(data.len() - Nonce::SIZE));

    // At this point we should have a buffer that's only the ciphertext.

    if let Some(key) = pw_key {
        let cipher = XChaCha20Poly1305::new(key.expose_secret());
        cipher
            .decrypt_in_place(&nonce.increment(), &[], data)
            .map_err(|_| Error::Password)?;
    }

    let cipher = XChaCha20Poly1305::new(key.expose_secret());
    cipher
        .decrypt_in_place(&nonce, &[], data)
        .map_err(|_| Error::SecretKey)?;

    Ok(())
}

#[must_use]
fn gen_key_nonce() -> (Secret<Key>, Nonce) {
    let mut rng = get_csrng();
    let mut key = GenericArray::default();
    rng.fill(key.as_mut_slice());
    let mut nonce = Nonce::default();
    rng.fill(nonce.as_mut_slice());
    (Secret::new(Key(key)), nonce)
}

// Type alias; to ensure that we're consistent on what the inner impl is.
type NonceImpl = XNonce;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
struct Nonce(NonceImpl);

impl Deref for Nonce {
    type Target = NonceImpl;

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
    const SIZE: usize = <NonceImpl as GenericSequence<_>>::Length::USIZE;

    #[must_use]
    pub fn increment(&self) -> Self {
        let mut inner = self.0;
        inner.as_mut_slice()[0] += 1;
        Self(inner)
    }

    #[must_use]
    pub fn from_slice(slice: &[u8]) -> Self {
        Self(*NonceImpl::from_slice(slice))
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Salt([u8; Self::SIZE]);

impl Salt {
    const SIZE: usize = argon2::password_hash::Salt::RECOMMENDED_LENGTH;

    fn random() -> Self {
        let mut salt = [0_u8; Self::SIZE];
        get_csrng().fill(&mut salt);
        Self(salt)
    }
}

impl AsRef<[u8]> for Salt {
    fn as_ref(&self) -> &[u8] {
        self.0.as_ref()
    }
}

/// Hashes an input to output a usable key.
fn kdf(password: &SecretVec<u8>) -> Result<(Secret<Key>, Salt), argon2::Error> {
    let salt = Salt::random();
    let hasher = get_argon2();
    let mut key = Key::default();
    hasher.hash_password_into(password.expose_secret().as_ref(), salt.as_ref(), &mut key)?;

    Ok((Secret::new(key), salt))
}

/// Returns Argon2id configured as follows:
///  - 15MiB of memory (`m`),
///  - an iteration count of 2 (`t`),
///  - and 2 degrees of parallelism (`p`).
///
/// This follows the [minimum recommended parameters suggested by OWASP][rec].
///
/// [rec]: https://link.eddie.sh/vaQ6a.
fn get_argon2() -> Argon2<'static> {
    let mut params = ParamsBuilder::new();
    params
        .m_cost(15 * 1024) // 15 MiB
        .expect("Hard coded params to work")
        .t_cost(2)
        .expect("Hard coded params to work")
        .p_cost(2)
        .expect("Hard coded params to work");
    let params = params.params().expect("Hard coded params to work");
    Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params)
}

/// Fetches a cryptographically secure random number generator. This indirection
/// is used for better auditing the quality of rng. Notably, this function
/// returns a `Rng` with the `CryptoRng` marker trait, preventing
/// non-cryptographically secure RNGs from being used.
#[must_use]
pub fn get_csrng() -> impl CryptoRng + Rng {
    rand::thread_rng()
}
