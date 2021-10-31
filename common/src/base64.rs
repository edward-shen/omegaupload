use base64::{DecodeError, URL_SAFE};

/// URL-safe Base64 encoding.
pub fn encode(input: impl AsRef<[u8]>) -> String {
    base64::encode_config(input, URL_SAFE)
}

/// URL-safe Base64 decoding.
///
/// # Errors
///
/// Returns an error if a buffer cannot be decoded, such as if there's an
/// incorrect number of bytes.
pub fn decode(input: impl AsRef<[u8]>) -> Result<Vec<u8>, DecodeError> {
    base64::decode_config(input, URL_SAFE)
}
