#![warn(clippy::nursery, clippy::pedantic)]

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

use std::str::FromStr;

use anyhow::{anyhow, bail, Context, Result};
use byte_unit::{n_mib_bytes, Byte};
use decrypt::{DecryptedData, MimeType};
use gloo_console::{error, log};
use http::uri::PathAndQuery;
use http::{StatusCode, Uri};
use js_sys::{Array, JsString, Object, Uint8Array};
use omegaupload_common::crypto::{Error as CryptoError, Key};
use omegaupload_common::secrecy::{Secret, SecretVec};
use omegaupload_common::{Expiration, PartialParsedUrl};
use reqwasm::http::Request;
use wasm_bindgen::prelude::{wasm_bindgen, Closure};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::{spawn_local, JsFuture};
use web_sys::{Event, IdbObjectStore, IdbOpenDbRequest, IdbTransactionMode, Location, Window};

use crate::decrypt::decrypt;
use crate::idb_object::IdbObject;
use crate::util::as_idb_db;

mod decrypt;
mod idb_object;
mod util;

const DOWNLOAD_SIZE_LIMIT: u128 = n_mib_bytes!(500);

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_name = loadFromDb)]
    pub fn load_from_db(mimetype: JsString);
    #[wasm_bindgen(js_name = renderMessage)]
    pub fn render_message(message: JsString);
}

fn window() -> Window {
    web_sys::window().expect("Failed to get a reference of the window")
}

fn location() -> Location {
    window().location()
}

fn open_idb() -> Result<IdbOpenDbRequest> {
    window()
        .indexed_db()
        .unwrap()
        .context("Missing browser idb impl")?
        .open("omegaupload")
        .map_err(|_| anyhow!("Failed to open idb"))
}

fn main() {
    std::panic::set_hook(Box::new(console_error_panic_hook::hook));

    if location().pathname().unwrap() == "/" {
        render_message("Go away".into());
        return;
    }

    render_message("Loading paste...".into());

    let url = String::from(location().to_string());
    let request_uri = {
        let mut uri_parts = url.parse::<Uri>().unwrap().into_parts();
        if let Some(parts) = uri_parts.path_and_query.as_mut() {
            *parts = PathAndQuery::from_str(&format!("/api{}", parts.path())).unwrap();
        }
        Uri::from_parts(uri_parts).unwrap()
    };

    let (key, needs_pw) = {
        let fragment = if let Some(fragment) = url.split_once('#').map(|(_, fragment)| fragment) {
            if fragment.is_empty() {
                error!("Key is missing in url; bailing.");
                render_message("Invalid paste link: Missing metadata.".into());
                return;
            }
            fragment
        } else {
            error!("Key is missing in url; bailing.");
            render_message("Invalid paste link: Missing metadata.".into());
            return;
        };

        let partial_parsed_url = match PartialParsedUrl::try_from(fragment) {
            Ok(partial_parsed_url) => partial_parsed_url,
            Err(e) => {
                error!("Failed to parse text fragment; bailing.");
                render_message(format!("Invalid paste link: {}", e).into());
                return;
            }
        };

        let key = if let Some(key) = partial_parsed_url.decryption_key {
            key
        } else {
            error!("Key is missing in url; bailing.");
            render_message("Invalid paste link: Missing decryption key.".into());
            return;
        };

        (key, partial_parsed_url.needs_password)
    };

    let password = if needs_pw {
        loop {
            let pw = window().prompt_with_message("A password is required to decrypt this paste:");

            match pw {
                // Ok button was entered.
                Ok(Some(password)) if !password.is_empty() => {
                    break Some(SecretVec::new(password.into_bytes()));
                }
                // Empty message was entered.
                Ok(Some(_)) => (),
                // Cancel button was entered.
                Ok(None) => {
                    render_message("This paste requires a password.".into());
                    return;
                }
                e => {
                    render_message("Internal error occurred.".into());
                    error!(format!("Error occurred at pw prompt: {:?}", e));
                    return;
                }
            }
        }
    } else {
        None
    };

    spawn_local(async move {
        if let Err(e) = fetch_resources(request_uri, key, password).await {
            log!(e.to_string());
        }
    });
}

#[allow(clippy::future_not_send)]
async fn fetch_resources(
    request_uri: Uri,
    key: Secret<Key>,
    password: Option<SecretVec<u8>>,
) -> Result<()> {
    match Request::get(&request_uri.to_string()).send().await {
        Ok(resp) if resp.status() == StatusCode::OK => {
            let expires = Expiration::try_from(resp.headers()).map_or_else(
                |_| "This item does not expire.".to_string(),
                |expires| expires.to_string(),
            );

            let data = {
                let data_fut = resp
                    .as_raw()
                    .array_buffer()
                    .expect("to get raw bytes from a response");
                let data = match JsFuture::from(data_fut).await {
                    Ok(data) => data,
                    Err(e) => {
                        render_message(
                            "Network failure: Failed to completely read encryption paste.".into(),
                        );
                        bail!(format!(
                            "JsFuture returned an error while fetching resp buffer: {:?}",
                            e
                        ));
                    }
                };
                Uint8Array::new(&data).to_vec()
            };

            if data.len() as u128 > DOWNLOAD_SIZE_LIMIT {
                render_message("The paste is too large to decrypt from the web browser. You must use the CLI tool to download this paste.".into());
                return Ok(());
            }

            let (decrypted, mimetype) = match decrypt(data, &key, password) {
                Ok(data) => data,
                Err(e) => {
                    let msg = match e {
                        CryptoError::Password => "The provided password was incorrect.",
                        CryptoError::SecretKey => "The secret key in the URL was incorrect.",
                        ref e => {
                            log!(format!("Bad kdf or corrupted blob: {}", e));
                            "An internal error occurred."
                        }
                    };

                    render_message(JsString::from(msg));
                    bail!(e);
                }
            };
            let db_open_req = open_idb()?;

            let on_success = Closure::once(Box::new(move |event| {
                on_success(&event, &decrypted, mimetype, &expires);
            }));

            db_open_req.set_onsuccess(Some(on_success.into_js_value().unchecked_ref()));
            db_open_req.set_onerror(Some(
                Closure::once(Box::new(|e: Event| log!(e)))
                    .into_js_value()
                    .unchecked_ref(),
            ));
            let on_upgrade = Closure::once(Box::new(move |event: Event| {
                let db = as_idb_db(&event);
                let _obj_store = db.create_object_store("decrypted data").unwrap();
            }));
            db_open_req.set_onupgradeneeded(Some(on_upgrade.into_js_value().unchecked_ref()));
        }
        Ok(resp) if resp.status() == StatusCode::NOT_FOUND => {
            render_message("Either the paste was burned or it never existed.".into());
        }
        Ok(resp) if resp.status() == StatusCode::BAD_REQUEST => {
            render_message("Invalid paste URL.".into());
        }
        Ok(err) => {
            render_message(err.status_text().into());
        }
        Err(err) => {
            render_message(format!("{}", err).into());
        }
    }

    Ok(())
}

fn on_success(event: &Event, decrypted: &DecryptedData, mimetype: MimeType, expires: &str) {
    let transaction: IdbObjectStore = as_idb_db(event)
        .transaction_with_str_and_mode("decrypted data", IdbTransactionMode::Readwrite)
        .unwrap()
        .object_store("decrypted data")
        .unwrap();

    let decrypted_object = match decrypted {
        DecryptedData::String(s) => IdbObject::new()
            .string()
            .expiration_text(expires)
            .data(&JsValue::from_str(s)),
        DecryptedData::Blob(blob) => IdbObject::new().blob().expiration_text(expires).data(blob),
        DecryptedData::Image(blob, size) => IdbObject::new()
            .image()
            .expiration_text(expires)
            .data(blob)
            .extra(
                "file_size",
                Byte::from_bytes(*size as u128)
                    .get_appropriate_unit(true)
                    .to_string(),
            ),
        DecryptedData::Audio(blob) => IdbObject::new().audio().expiration_text(expires).data(blob),
        DecryptedData::Video(blob) => IdbObject::new().video().expiration_text(expires).data(blob),
        DecryptedData::Archive(blob, entries) => IdbObject::new()
            .archive()
            .expiration_text(expires)
            .data(blob)
            .extra(
                "entries",
                JsValue::from(
                    entries
                        .iter()
                        .filter_map(|x| JsValue::from_serde(x).ok())
                        .collect::<Array>(),
                ),
            ),
    };

    let put_action = transaction
        .put_with_key(
            &Object::from(decrypted_object),
            &JsString::from(location().pathname().unwrap()),
        )
        .unwrap();
    put_action.set_onsuccess(Some(
        Closure::once(Box::new(|| {
            log!("success");
            load_from_db(JsString::from(mimetype.0));
        }))
        .into_js_value()
        .unchecked_ref(),
    ));
    put_action.set_onerror(Some(
        Closure::once(Box::new(|e: Event| log!(e)))
            .into_js_value()
            .unchecked_ref(),
    ));
}
