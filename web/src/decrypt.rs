use std::io::Cursor;
use std::sync::Arc;

use gloo_console::log;
use js_sys::{Array, Uint8Array};
use omegaupload_common::crypto::{open_in_place, Error, Key};
use omegaupload_common::secrecy::{Secret, SecretVec};
use serde::Serialize;
use wasm_bindgen::JsCast;
use web_sys::{Blob, BlobPropertyBag};

#[derive(Clone, Serialize)]
pub struct ArchiveMeta {
    name: String,
    file_size: u64,
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
    key: &Secret<Key>,
    maybe_password: Option<SecretVec<u8>>,
) -> Result<DecryptedData, Error> {
    open_in_place(&mut container, key, maybe_password)?;

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
        if let Ok(string) = String::from_utf8(container) {
            Ok(DecryptedData::String(Arc::new(string)))
        } else {
            Ok(DecryptedData::Blob(blob))
        }
    } else if mime_type.starts_with("image/")
        // application/x-riff is WebP
        || mime_type == "application/x-riff"
    {
        Ok(DecryptedData::Image(blob, container.len()))
    } else if mime_type.starts_with("audio/") {
        Ok(DecryptedData::Audio(blob))
    } else if mime_type.starts_with("video/")
        // application/x-matroska is mkv
        || mime_type == "application/x-matroska"
    {
        Ok(DecryptedData::Video(blob))
    } else if mime_type == "application/zip" {
        let mut entries = vec![];
        let cursor = Cursor::new(container);
        if let Ok(mut zip) = zip::ZipArchive::new(cursor) {
            for i in 0..zip.len() {
                match zip.by_index(i) {
                    Ok(file) => entries.push(ArchiveMeta {
                        name: file.name().to_string(),
                        file_size: file.size(),
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
