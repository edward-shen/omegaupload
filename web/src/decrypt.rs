use std::fmt::{Display, Formatter};
use std::io::Cursor;
use std::sync::Arc;

use gloo_console::log;
use image::{EncodableLayout, GenericImageView, ImageDecoder};
use js_sys::{Array, Uint8Array};
use omegaupload_common::crypto::{open_in_place, Key, Nonce};
use wasm_bindgen::JsCast;
use web_sys::Blob;

#[derive(Clone)]
pub enum DecryptedData {
    String(Arc<String>),
    Blob(Arc<Blob>),
    Image(Arc<Blob>, (u32, u32), usize),
    Audio(Arc<Blob>),
    Video(Arc<Blob>),
}

fn now() -> f64 {
    web_sys::window()
        .expect("should have a Window")
        .performance()
        .expect("should have a Performance")
        .now()
}

pub fn decrypt(
    mut container: Vec<u8>,
    key: Key,
    nonce: Nonce,
    maybe_password: Option<Key>,
) -> Result<DecryptedData, PasteCompleteConstructionError> {
    let container = &mut container;
    log!("Stage 1 decryption started.");
    let start = now();
    if let Some(password) = maybe_password {
        open_in_place(container, &nonce.increment(), &password)
            .map_err(|_| PasteCompleteConstructionError::StageOneFailure)?;
    }
    log!(format!("Stage 1 completed in {}ms", now() - start));

    log!("Stage 2 decryption started.");
    let start = now();
    open_in_place(container, &nonce, &key)
        .map_err(|_| PasteCompleteConstructionError::StageTwoFailure)?;
    log!(format!("Stage 2 completed in {}ms", now() - start));

    if let Ok(decrypted) = std::str::from_utf8(container) {
        Ok(DecryptedData::String(Arc::new(decrypted.to_owned())))
    } else {
        log!("Blob conversion started.");
        let start = now();
        let blob_chunks = Array::new_with_length(container.chunks(65536).len().try_into().unwrap());
        for (i, chunk) in container.chunks(65536).enumerate() {
            let array = Uint8Array::new_with_length(chunk.len().try_into().unwrap());
            array.copy_from(chunk);
            blob_chunks.set(i.try_into().unwrap(), array.dyn_into().unwrap());
        }
        let blob =
            Arc::new(Blob::new_with_u8_array_sequence(blob_chunks.dyn_ref().unwrap()).unwrap());
        log!(format!("Blob conversion completed in {}ms", now() - start));

        log!("Image introspection started");
        let start = now();
        let res = image::guess_format(&container);
        log!(format!(
            "Image introspection completed in {}ms",
            now() - start
        ));
        // let image_reader = image::io::Reader::new(Cursor::new(container.as_bytes()));
        if let Ok(dimensions) = res {
            log!(format!("{:?}", dimensions));
            Ok(DecryptedData::Image(blob, (0, 0), container.len()))
        } else {
            let mime_type = tree_magic_mini::from_u8(container);
            log!(mime_type);
            if mime_type.starts_with("audio/") {
                Ok(DecryptedData::Audio(blob))
            } else if mime_type.starts_with("video/") || mime_type.ends_with("x-matroska") {
                Ok(DecryptedData::Video(blob))
            } else {
                Ok(DecryptedData::Blob(blob))
            }
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
