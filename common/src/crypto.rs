use std::fmt::Display;
use std::ops::{Deref, DerefMut};

use argon2::Argon2;
use chacha20poly1305::aead::generic_array::sequence::GenericSequence;
use chacha20poly1305::aead::generic_array::GenericArray;
use chacha20poly1305::aead::{AeadInPlace, NewAead};
use chacha20poly1305::XChaCha20Poly1305;
use chacha20poly1305::XNonce;
use rand::{thread_rng, Rng};
use typenum::Unsigned;

pub use chacha20poly1305::Key;

#[derive(Debug)]
pub enum Error {
    ChaCha20Poly1305(chacha20poly1305::aead::Error),
    Argon2(argon2::Error),
}

impl From<chacha20poly1305::aead::Error> for Error {
    fn from(err: chacha20poly1305::aead::Error) -> Self {
        Error::ChaCha20Poly1305(err)
    }
}

impl From<argon2::Error> for Error {
    fn from(err: argon2::Error) -> Self {
        Error::Argon2(err)
    }
}

impl std::error::Error for Error {}

impl Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::ChaCha20Poly1305(_) => write!(f, "Decryption failed"),
            Error::Argon2(_) => write!(f, "KDF failed"),
        }
    }
}

/// Seals the provided message with an optional message. The resulting sealed
/// message has the nonce used to encrypt the message appended to it as well as
/// a salt string used to derive the key. In other words, the modified buffer is
/// one of the following to possibilities, depending if there was a password
/// provided:
///
/// ```
/// modified = C(message, rng_key, nonce) || nonce
/// ```
/// or
/// ```
/// modified = C(C(message, rng_key, nonce), kdf(pw, salt), nonce + 1) || nonce || salt
/// ```
///
/// Where:
///  - `C(message, key, nonce)` represents encrypting a provided message with
///     XChaCha20Poly1305.
///  - `rng_key` represents a randomly generated key.
///  - `kdf(pw, salt)` represents a key derived from Argon2.
pub fn seal_in_place(message: &mut Vec<u8>, pw: Option<&str>) -> Result<Key, Error> {
    let (key, nonce) = gen_key_nonce();
    let cipher = XChaCha20Poly1305::new(&key);
    cipher.encrypt_in_place(&nonce, &[], message)?;

    let mut maybe_salt_string = None;
    if let Some(password) = pw {
        let (key, salt_string) = kdf(&password)?;
        maybe_salt_string = Some(salt_string);
        let cipher = XChaCha20Poly1305::new(&key);
        cipher.encrypt_in_place(&nonce.increment(), &[], message)?;
    }

    message.extend_from_slice(nonce.as_slice());
    if let Some(maybe_salted_string) = maybe_salt_string {
        message.extend_from_slice(maybe_salted_string.as_ref());
    }
    Ok(key)
}

pub fn open_in_place(data: &mut Vec<u8>, key: &Key, password: Option<&str>) -> Result<(), Error> {
    let buffer_len = data.len();
    let pw_key = if let Some(password) = password {
        let salt_buf = data.split_off(buffer_len - Salt::SIZE);
        let argon = Argon2::default();
        let mut pw_key = Key::default();
        argon.hash_password_into(password.as_bytes(), &salt_buf, &mut pw_key)?;
        Some(pw_key)
    } else {
        None
    };

    let nonce = Nonce::from_slice(&data.split_off(Nonce::SIZE));

    // At this point we should have a buffer that's only the ciphertext.

    if let Some(key) = pw_key {
        let cipher = XChaCha20Poly1305::new(&key);
        cipher.decrypt_in_place(&nonce.increment(), &[], data)?;
    }

    let cipher = XChaCha20Poly1305::new(&key);
    cipher.decrypt_in_place(&nonce, &[], data)?;

    Ok(())
}

/// Securely generates a random key and nonce.
#[must_use]
fn gen_key_nonce() -> (Key, Nonce) {
    let mut rng = thread_rng();
    let mut key: Key = GenericArray::default();
    rng.fill(key.as_mut_slice());
    let mut nonce = Nonce::default();
    rng.fill(nonce.as_mut_slice());
    (key, nonce)
}

// Type alias; to ensure that we're consistent on what the inner impl is.
type NonceImpl = XNonce;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Nonce(NonceImpl);

impl Default for Nonce {
    fn default() -> Self {
        Self(GenericArray::default())
    }
}

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
struct Salt([u8; Salt::SIZE]);

impl Salt {
    const SIZE: usize = argon2::password_hash::Salt::RECOMMENDED_LENGTH;

    fn random() -> Self {
        let mut salt = [0u8; Salt::SIZE];
        thread_rng().fill(&mut salt);
        Self(salt)
    }
}

impl AsRef<[u8]> for Salt {
    fn as_ref(&self) -> &[u8] {
        self.0.as_ref()
    }
}

/// Hashes an input to output a usable key.
fn kdf(password: &str) -> Result<(Key, Salt), argon2::Error> {
    let salt = Salt::random();
    let hasher = Argon2::default();
    let mut key = Key::default();
    hasher.hash_password_into(password.as_ref(), salt.as_ref(), &mut key)?;

    Ok((*Key::from_slice(&key), salt))
}
