// OmegaUpload Web Frontend
// Copyright (C) 2021  Edward Shen
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

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

pub struct MimeType(pub String);

pub fn decrypt(
    mut container: Vec<u8>,
    key: &Secret<Key>,
    maybe_password: Option<SecretVec<u8>>,
) -> Result<(DecryptedData, MimeType), Error> {
    open_in_place(&mut container, key, maybe_password)?;

    let mime_type = tree_magic_mini::from_u8(&container);
    log!("Mime type: ", mime_type);

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

    let data = match container.content_type() {
        ContentType::Text => DecryptedData::String(Arc::new(
            // SAFETY: ContentType::Text is guaranteed to be valid UTF-8.
            unsafe { String::from_utf8_unchecked(container) },
        )),
        ContentType::Image => DecryptedData::Image(blob, container.len()),
        ContentType::Audio => DecryptedData::Audio(blob),
        ContentType::Video => DecryptedData::Video(blob),
        ContentType::ZipArchive => {
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
            DecryptedData::Archive(blob, entries)
        }
        ContentType::GzipArchive => {
            let mut entries = vec![];
            let cursor = Cursor::new(container);
            let gzip_dec = flate2::read::GzDecoder::new(cursor);
            let mut archive = tar::Archive::new(gzip_dec);
            if let Ok(files) = archive.entries() {
                for file in files {
                    if let Ok(file) = file {
                        let file_path = if let Ok(file_path) = file.path() {
                            file_path.display().to_string()
                        } else {
                            "<Invalid utf-8 path>".to_string()
                        };
                        entries.push(ArchiveMeta {
                            name: file_path,
                            file_size: file.size(),
                        });
                    }
                }
            }
            if entries.len() > 0 {
                DecryptedData::Archive(blob, entries)
            } else {
                DecryptedData::Blob(blob)
            }
        },
        ContentType::Unknown => DecryptedData::Blob(blob),
    };

    Ok((data, MimeType(mime_type.to_owned())))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum ContentType {
    Text,
    Image,
    Audio,
    Video,
    ZipArchive,
    GzipArchive,
    Unknown,
}

trait ContentTypeExt {
    fn mime_type(&self) -> &str;
    fn content_type(&self) -> ContentType;
}

impl<T: AsRef<[u8]>> ContentTypeExt for T {
    fn mime_type(&self) -> &str {
        tree_magic_mini::from_u8(self.as_ref())
    }

    fn content_type(&self) -> ContentType {
        let mime_type = self.mime_type();
        // check image first; tree magic match_u8 matches SVGs as plain text
        if mime_type.starts_with("image/")
            // application/x-riff is WebP
            || mime_type == "application/x-riff"
        {
            ContentType::Image
        } else if tree_magic_mini::match_u8("text/plain", self.as_ref()) {
            if std::str::from_utf8(self.as_ref()).is_ok() {
                ContentType::Text
            } else {
                ContentType::Unknown
            }
        } else if mime_type.starts_with("audio/") {
            ContentType::Audio
        } else if mime_type.starts_with("video/")
            // application/x-matroska is mkv
            || mime_type == "application/x-matroska"
        {
            ContentType::Video
        } else if mime_type == "application/zip" {
            ContentType::ZipArchive
        } else if mime_type == "application/gzip" {
            ContentType::GzipArchive
        } else {
            ContentType::Unknown
        }
    }
}

#[cfg(test)]
mod content_type {
    use super::*;

    macro_rules! test_content_type {
        ($($name:ident, $path:literal, $type:expr),*) => {
            $(
                #[test]
                fn $name() {
                    let data = include_bytes!(concat!("../../test/", $path));
                    assert_eq!(data.content_type(), $type);
                }
            )*
        };
    }

    test_content_type!(license_is_text, "LICENSE.md", ContentType::Text);
    test_content_type!(code_is_text, "code.rs", ContentType::Text);
    test_content_type!(patch_is_text, "0000-test-patch.patch", ContentType::Text);
    test_content_type!(png_is_image, "image.png", ContentType::Image);
    test_content_type!(webp_is_image, "image.webp", ContentType::Image);
    test_content_type!(svg_is_image, "image.svg", ContentType::Image);
    test_content_type!(mp3_is_audio, "music.mp3", ContentType::Audio);
    test_content_type!(mp4_is_video, "movie.mp4", ContentType::Video);
    test_content_type!(mkv_is_video, "movie.mkv", ContentType::Video);
    test_content_type!(zip_is_zip, "archive.zip", ContentType::ZipArchive);
    test_content_type!(gzip_is_gzip, "image.png.gz", ContentType::GzipArchive);
    test_content_type!(binary_is_unknown, "omegaupload", ContentType::Unknown);
    test_content_type!(pgp_is_text, "text.pgp", ContentType::Text);
}
