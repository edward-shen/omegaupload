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
use js_sys::{Array, JsString, Object};
use omegaupload_common::base64;
use omegaupload_common::crypto::seal_in_place;
use omegaupload_common::crypto::{Error as CryptoError, Key};
use omegaupload_common::fragment::Builder;
use omegaupload_common::secrecy::{ExposeSecret, Secret, SecretString, SecretVec};
use omegaupload_common::{Expiration, PartialParsedUrl, Url};
use wasm_bindgen::prelude::{wasm_bindgen, Closure};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::spawn_local;
use web_sys::{Event, IdbObjectStore, IdbOpenDbRequest, IdbTransactionMode, Location, Window};

use crate::decrypt::decrypt;
use crate::idb_object::IdbObject;
use crate::util::as_idb_db;

mod decrypt;
mod idb_object;
mod util;

const DOWNLOAD_SIZE_LIMIT: u128 = n_mib_bytes!(500);

#[wasm_bindgen(raw_module = "../src/render")]
extern "C" {
    #[wasm_bindgen(js_name = loadFromDb)]
    pub fn load_from_db(mime_type: JsString, name: Option<JsString>, language: Option<JsString>);
    #[wasm_bindgen(js_name = renderMessage)]
    pub fn render_message(message: JsString);
    #[wasm_bindgen(js_name = createUploadUi)]
    pub fn create_upload_ui();
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

#[wasm_bindgen]
#[allow(clippy::missing_panics_doc)]
pub fn start() {
    std::panic::set_hook(Box::new(console_error_panic_hook::hook));

    if location().pathname().unwrap() == "/" {
        create_upload_ui();
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

    let (
        key,
        PartialParsedUrl {
            needs_password,
            name,
            language,
            ..
        },
    ) = {
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

        let mut partial_parsed_url = match PartialParsedUrl::try_from(fragment) {
            Ok(partial_parsed_url) => partial_parsed_url,
            Err(e) => {
                error!("Failed to parse text fragment; bailing.");
                render_message(format!("Invalid paste link: {e}").into());
                return;
            }
        };

        let key = if let Some(key) = partial_parsed_url.decryption_key.take() {
            key
        } else {
            error!("Key is missing in url; bailing.");
            render_message("Invalid paste link: Missing decryption key.".into());
            return;
        };

        (key, partial_parsed_url)
    };

    let password = if needs_password {
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
                    error!(format!("Error occurred at pw prompt: {e:?}"));
                    return;
                }
            }
        }
    } else {
        None
    };

    spawn_local(async move {
        if let Err(e) = fetch_resources(request_uri, key, password, name, language).await {
            log!(e.to_string());
        }
    });
}

#[wasm_bindgen]
#[allow(clippy::future_not_send)]
pub async fn encrypt_array_buffer(location: String, data: Vec<u8>) -> Result<JsString, JsString> {
    do_encrypt(location, data).await.map_err(|e| {
        log!(format!("[rs] Error encrypting array buffer: {}", e));
        JsString::from(e.to_string())
    })
}

#[allow(clippy::future_not_send)]
async fn do_encrypt(location: String, mut data: Vec<u8>) -> Result<JsString> {
    let (data, key) = {
        let enc_key = seal_in_place(&mut data, None)?;
        let key = SecretString::new(base64::encode(&enc_key.expose_secret().as_ref()));
        (data, key)
    };

    let mut url = Url::from_str(&location)?;
    let fragment = Builder::new(key);

    let short_code = reqwest::Client::new()
        .post(url.as_ref())
        .body(data)
        .send()
        .await?
        .text()
        .await?;

    url.set_path(&short_code);
    url.set_fragment(Some(fragment.build().expose_secret()));

    Ok(JsString::from(url.as_ref()))
}

#[allow(clippy::future_not_send)]
async fn fetch_resources(
    request_uri: Uri,
    key: Secret<Key>,
    password: Option<SecretVec<u8>>,
    name: Option<String>,
    language: Option<String>,
) -> Result<()> {
    match reqwest::Client::new()
        .get(&request_uri.to_string())
        .send()
        .await
    {
        Ok(resp) if resp.status() == StatusCode::OK => {
            let expires = resp
                .headers()
                .get(http::header::EXPIRES)
                .and_then(|header| Expiration::try_from(header).ok())
                .map_or_else(
                    || "This item does not expire.".to_string(),
                    |expires| expires.to_string(),
                );

            let data = resp
                .bytes()
                .await
                .expect("to get raw bytes from a response")
                .to_vec();

            if data.len() as u128 > DOWNLOAD_SIZE_LIMIT {
                render_message("The paste is too large to decrypt from the web browser. You must use the CLI tool to download this paste.".into());
                return Ok(());
            }

            let (decrypted, mimetype) = match decrypt(data, &key, password, name.as_deref()) {
                Ok(data) => data,
                Err(e) => {
                    let msg = match e {
                        CryptoError::Password => "The provided password was incorrect.",
                        CryptoError::SecretKey => "The secret key in the URL was incorrect.",
                        ref e => {
                            log!(format!("Bad kdf or corrupted blob: {e}"));
                            "An internal error occurred."
                        }
                    };

                    render_message(JsString::from(msg));
                    bail!(e);
                }
            };
            let db_open_req = open_idb()?;

            let on_success = Closure::once(Box::new(move |event| {
                on_success(&event, &decrypted, mimetype, &expires, name, language);
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
            render_message(err.status().as_str().into());
        }
        Err(err) => {
            render_message(format!("{err}").into());
        }
    }

    Ok(())
}

fn on_success(
    event: &Event,
    decrypted: &DecryptedData,
    mimetype: MimeType,
    expires: &str,
    name: Option<String>,
    language: Option<String>,
) {
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
            log!("[rs] Successfully inserted encrypted item into storage.");
            let name = name.map(JsString::from);
            let language = language.map(JsString::from);
            load_from_db(JsString::from(mimetype.0), name, language);
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
