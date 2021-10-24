#![warn(clippy::nursery, clippy::pedantic)]

use std::str::FromStr;

use anyhow::{anyhow, Context, Result};
use byte_unit::Byte;
use decrypt::DecryptedData;
use gloo_console::log;
use http::uri::PathAndQuery;
use http::{StatusCode, Uri};
use js_sys::{JsString, Object, Uint8Array};
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

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_name = loadFromDb)]
    fn load_from_db();
    #[wasm_bindgen(js_name = createNotFoundUi)]
    fn create_not_found_ui();
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
    if location().pathname().unwrap() == "/" {
    } else {
        spawn_local(async {
            if let Err(e) = fetch_resources(request_uri, url).await {
                log!(e.to_string());
            }
        });
    }
}

#[allow(clippy::future_not_send)]
async fn fetch_resources(request_uri: Uri, url: String) -> Result<()> {
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
                    .expect("Failed to get raw bytes from response");
                let data = JsFuture::from(data_fut)
                    .await
                    .expect("Failed to result array buffer future");
                Uint8Array::new(&data).to_vec()
            };

            let (key, nonce) = {
                let partial_parsed_url = url
                    .split_once('#')
                    .map(|(_, fragment)| PartialParsedUrl::from(fragment))
                    .unwrap_or_default();
                let key = partial_parsed_url
                    .decryption_key
                    .context("missing key should be handled in the future")?;
                let nonce = partial_parsed_url
                    .nonce
                    .context("missing nonce be handled in the future")?;
                (key, nonce)
            };

            let decrypted = decrypt(data, key, nonce, None)?;
            let db_open_req = open_idb()?;

            // On success callback
            let on_success = Closure::once(Box::new(move |event: Event| {
                let transaction: IdbObjectStore = as_idb_db(&event)
                    .transaction_with_str_and_mode("decrypted data", IdbTransactionMode::Readwrite)
                    .unwrap()
                    .object_store("decrypted data")
                    .unwrap();

                log!(line!());

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

                log!(line!());
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
                        log!(line!());
                        log!(e);
                    }) as Box<dyn Fn(Event)>)
                    .into_js_value()
                    .unchecked_ref(),
                ));
            }) as Box<dyn FnOnce(Event)>);

            db_open_req.set_onsuccess(Some(on_success.into_js_value().unchecked_ref()));
            db_open_req.set_onerror(Some(
                Closure::wrap(Box::new(|e| {
                    log!(line!());
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
            log!(&db_open_req);
        }
        Ok(resp) if resp.status() == StatusCode::NOT_FOUND => {
            create_not_found_ui();
        }
        Ok(resp) if resp.status() == StatusCode::BAD_REQUEST => {}
        Ok(err) => {}
        Err(err) => {}
    }

    Ok(())
}
