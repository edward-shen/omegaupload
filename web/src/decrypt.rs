use std::fmt::{Display, Formatter};
use std::io::Cursor;
use std::sync::Arc;

use gloo_console::log;
use js_sys::{Array, Uint8Array};
use omegaupload_common::crypto::{open_in_place, Key, Nonce};
use serde::Serialize;
use wasm_bindgen::JsCast;
use web_sys::{Blob, BlobPropertyBag};

#[derive(Clone, Serialize)]
pub struct ArchiveMeta {
    name: String,
    file_size: usize,
}

#[derive(Clone)]
pub enum DecryptedData {
    String(Arc<String>),
    Blob(Arc<Blob>),
    Image(Arc<Blob>, usize),
    Audio(Arc<Blob>),
    Video(Arc<Blob>),
    Archive(Arc<Blob>, Vec<ArchiveMeta>),
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
    log!("Stage 1 decryption started.");
    let start = now();

    if let Some(password) = maybe_password {
        crate::render_message("Decrypting Stage 1...".into());
        open_in_place(&mut container, &nonce.increment(), &password).map_err(|_| {
            crate::render_message("Unable to decrypt paste with the provided password.".into());
            PasteCompleteConstructionError::StageOneFailure
        })?;
    }
    log!(format!("Stage 1 completed in {}ms", now() - start));

    log!("Stage 2 decryption started.");
    let start = now();
    crate::render_message("Decrypting Stage 2...".into());
    open_in_place(&mut container, &nonce, &key).map_err(|_| {
        crate::render_message(
            "Unable to decrypt paste with the provided encryption key and nonce.".into(),
        );
        PasteCompleteConstructionError::StageTwoFailure
    })?;
    log!(format!("Stage 2 completed in {}ms", now() - start));

    let mime_type = tree_magic_mini::from_u8(&container);
    log!("Mimetype: ", mime_type);

    log!("Blob conversion started.");
    let start = now();
    let blob_chunks = Array::new_with_length(container.chunks(65536).len().try_into().unwrap());
    for (i, chunk) in container.chunks(65536).enumerate() {
        let array = Uint8Array::new_with_length(chunk.len().try_into().unwrap());
        array.copy_from(chunk);
        blob_chunks.set(i.try_into().unwrap(), array.dyn_into().unwrap());
    }
    let mut blob_props = BlobPropertyBag::new();
    blob_props.type_(mime_type);
    let blob = Arc::new(
        Blob::new_with_u8_array_sequence_and_options(blob_chunks.dyn_ref().unwrap(), &blob_props)
            .unwrap(),
    );
    log!(format!("Blob conversion completed in {}ms", now() - start));

    if mime_type.starts_with("text/") {
        String::from_utf8(container)
            .map(Arc::new)
            .map(DecryptedData::String)
            .map_err(|_| PasteCompleteConstructionError::InvalidEncoding)
    } else if mime_type.starts_with("image/") || mime_type == "application/x-riff" {
        Ok(DecryptedData::Image(blob, container.len()))
    } else if mime_type.starts_with("audio/") {
        Ok(DecryptedData::Audio(blob))
    } else if mime_type.starts_with("video/") || mime_type == "application/x-matroska" {
        Ok(DecryptedData::Video(blob))
    } else if mime_type == "application/zip" {
        let mut entries = vec![];
        let cursor = Cursor::new(container);
        if let Ok(mut zip) = zip::ZipArchive::new(cursor) {
            for i in 0..zip.len() {
                match zip.by_index(i) {
                    Ok(file) => entries.push(ArchiveMeta {
                        name: file.name().to_string(),
                        file_size: file.size() as usize,
                    }),
                    Err(err) => match err {
                        zip::result::ZipError::UnsupportedArchive(s) => {
                            log!("Unsupported: ", s.to_string());
                        }
                        _ => {
                            log!(format!("Error: {}", err));
                        }
                    },
                }
            }
        }

        entries.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(DecryptedData::Archive(blob, entries))
    } else if mime_type == "application/gzip" {
        Ok(DecryptedData::Archive(blob, vec![]))
    } else {
        Ok(DecryptedData::Blob(blob))
    }
}

#[derive(Debug)]
pub enum PasteCompleteConstructionError {
    StageOneFailure,
    StageTwoFailure,
    InvalidEncoding,
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
            PasteCompleteConstructionError::InvalidEncoding => write!(
                f,
                "Got an file with a text/* mime type, but was unable to parsed as valid UTF-8?"
            ),
        }
    }
}
