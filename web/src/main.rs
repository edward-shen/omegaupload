#![warn(clippy::nursery, clippy::pedantic)]

use std::str::FromStr;

use anyhow::{anyhow, bail, Context, Result};
use byte_unit::{n_mib_bytes, Byte};
use decrypt::DecryptedData;
use gloo_console::{error, log};
use http::uri::PathAndQuery;
use http::{StatusCode, Uri};
use js_sys::{JsString, Object, Uint8Array};
use omegaupload_common::crypto::{Key, Nonce};
use omegaupload_common::{hash, Expiration, PartialParsedUrl};
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
    pub fn load_from_db();
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

    render_message("Loading paste...".into());

    let url = String::from(location().to_string());
    let request_uri = {
        let mut uri_parts = url.parse::<Uri>().unwrap().into_parts();
        if let Some(parts) = uri_parts.path_and_query.as_mut() {
            *parts = PathAndQuery::from_str(&format!("/api{}", parts.path())).unwrap();
        }
        Uri::from_parts(uri_parts).unwrap()
    };

    log!(&url);
    log!(&request_uri.to_string());
    log!(&location().pathname().unwrap());
    let (key, nonce, needs_pw) = {
        let partial_parsed_url = url
            .split_once('#')
            .map(|(_, fragment)| PartialParsedUrl::from(fragment))
            .unwrap_or_default();
        let key = match partial_parsed_url.decryption_key {
            Some(key) => key,
            None => {
                error!("Key is missing in url; bailing.");
                render_message("Invalid paste link: Missing decryption key.".into());
                return;
            }
        };
        let nonce = match partial_parsed_url.nonce {
            Some(nonce) => nonce,
            None => {
                error!("Nonce is missing in url; bailing.");
                render_message("Invalid paste link: Missing nonce.".into());
                return;
            }
        };
        (key, nonce, partial_parsed_url.needs_password)
    };

    let password = if needs_pw {
        loop {
            let pw = window().prompt_with_message("A password is required to decrypt this paste:");

            if let Ok(Some(password)) = pw {
                if !password.is_empty() {
                    break Some(hash(password));
                }
            }
        }
    } else {
        None
    };

    if location().pathname().unwrap() == "/" {
    } else {
        spawn_local(async move {
            if let Err(e) = fetch_resources(request_uri, key, nonce, password).await {
                log!(e.to_string());
            }
        });
    }
}

#[allow(clippy::future_not_send)]
async fn fetch_resources(
    request_uri: Uri,
    key: Key,
    nonce: Nonce,
    password: Option<Key>,
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

            let decrypted = decrypt(data, key, nonce, password)?;
            let db_open_req = open_idb()?;

            // On success callback
            let on_success = Closure::once(Box::new(move |event: Event| {
                let transaction: IdbObjectStore = as_idb_db(&event)
                    .transaction_with_str_and_mode("decrypted data", IdbTransactionMode::Readwrite)
                    .unwrap()
                    .object_store("decrypted data")
                    .unwrap();

                let decrypted_object = match &decrypted {
                    DecryptedData::String(s) => IdbObject::new()
                        .string()
                        .expiration_text(&expires)
                        .data(&JsValue::from_str(s)),
                    DecryptedData::Blob(blob) => {
                        IdbObject::new().blob().expiration_text(&expires).data(blob)
                    }
                    DecryptedData::Image(blob, (width, height), size) => IdbObject::new()
                        .image()
                        .expiration_text(&expires)
                        .data(blob)
                        .extra("width", *width)
                        .extra("height", *height)
                        .extra(
                            "button",
                            &format!(
                                "Download {} \u{2014} {} by {}",
                                Byte::from_bytes(*size as u128).get_appropriate_unit(true),
                                width,
                                height,
                            ),
                        ),
                    DecryptedData::Audio(blob) => IdbObject::new()
                        .audio()
                        .expiration_text(&expires)
                        .data(blob),
                    DecryptedData::Video(blob) => IdbObject::new()
                        .video()
                        .expiration_text(&expires)
                        .data(blob),
                };

                let put_action = transaction
                    .put_with_key(
                        &Object::from(decrypted_object),
                        &JsString::from(location().pathname().unwrap()),
                    )
                    .unwrap();
                put_action.set_onsuccess(Some(
                    Closure::wrap(Box::new(|| {
                        log!("success");
                        load_from_db();
                    }) as Box<dyn Fn()>)
                    .into_js_value()
                    .unchecked_ref(),
                ));
                put_action.set_onerror(Some(
                    Closure::wrap(Box::new(|e| {
                        log!(e);
                    }) as Box<dyn Fn(Event)>)
                    .into_js_value()
                    .unchecked_ref(),
                ));
            }) as Box<dyn FnOnce(Event)>);

            db_open_req.set_onsuccess(Some(on_success.into_js_value().unchecked_ref()));
            db_open_req.set_onerror(Some(
                Closure::wrap(Box::new(|e| {
                    log!(e);
                }) as Box<dyn Fn(Event)>)
                .into_js_value()
                .unchecked_ref(),
            ));
            let on_upgrade = Closure::wrap(Box::new(move |event: Event| {
                let db = as_idb_db(&event);
                let _ = db.create_object_store("decrypted data").unwrap();
            }) as Box<dyn FnMut(Event)>);
            db_open_req.set_onupgradeneeded(Some(on_upgrade.into_js_value().unchecked_ref()));
        }
        Ok(resp) if resp.status() == StatusCode::NOT_FOUND => {
            render_message("Either the paste was burned or it never existed.".into());
        }
        Ok(resp) if resp.status() == StatusCode::BAD_REQUEST => {
            render_message("Invalid paste URL.".into());
        }
        Ok(err) => {
            render_message(format!("{}", err.status_text()).into());
        }
        Err(err) => {
            render_message(format!("{}", err).into());
        }
    }

    Ok(())
}
