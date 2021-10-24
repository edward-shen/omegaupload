use std::fmt::{Display, Formatter};
use std::sync::Arc;

use gloo_console::log;
use image::GenericImageView;
use js_sys::{Array, Uint8Array};
use omegaupload_common::crypto::{open_in_place, Key, Nonce};
use wasm_bindgen::JsCast;
use web_sys::Blob;

use crate::DecryptedData;

pub fn decrypt(
    mut container: Vec<u8>,
    key: Key,
    nonce: Nonce,
    maybe_password: Option<Key>,
) -> Result<DecryptedData, PasteCompleteConstructionError> {
    let container = &mut container;
    log!("stage 1 decryption start");
    if let Some(password) = maybe_password {
        open_in_place(container, &nonce.increment(), &password)
            .map_err(|_| PasteCompleteConstructionError::StageOneFailure)?;
    }

    log!("stage 2 decryption start");
    open_in_place(container, &nonce, &key)
        .map_err(|_| PasteCompleteConstructionError::StageTwoFailure)?;

    log!("stage 2 decryption end");
    if let Ok(decrypted) = std::str::from_utf8(&container) {
        Ok(DecryptedData::String(Arc::new(decrypted.to_owned())))
    } else {
        log!("blob conversion start");
        let blob_chunks = Array::new_with_length(container.chunks(65536).len().try_into().unwrap());
        for (i, chunk) in container.chunks(65536).enumerate() {
            let array = Uint8Array::new_with_length(chunk.len().try_into().unwrap());
            array.copy_from(&chunk);
            blob_chunks.set(i.try_into().unwrap(), array.dyn_into().unwrap());
        }
        let blob =
            Arc::new(Blob::new_with_u8_array_sequence(blob_chunks.dyn_ref().unwrap()).unwrap());
        log!("blob conversion end");

        if let Ok(image) = image::load_from_memory(&container) {
            Ok(DecryptedData::Image(
                blob,
                image.dimensions(),
                container.len(),
            ))
        } else {
            Ok(DecryptedData::Blob(blob))
        }
    }
}

#[derive(Debug)]
pub enum PasteCompleteConstructionError {
    StageOneFailure,
    StageTwoFailure,
}

impl std::error::Error for PasteCompleteConstructionError {}

impl Display for PasteCompleteConstructionError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            PasteCompleteConstructionError::StageOneFailure => {
                write!(f, "Failed to decrypt stage one.")
            }
            PasteCompleteConstructionError::StageTwoFailure => {
                write!(f, "Failed to decrypt stage two.")
            }
        }
    }
}
