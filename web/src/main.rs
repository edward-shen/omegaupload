#![warn(clippy::nursery, clippy::pedantic)]

use std::str::FromStr;

use byte_unit::Byte;
use decrypt::DecryptedData;
use gloo_console::log;
use http::header::EXPIRES;
use http::uri::PathAndQuery;
use http::{StatusCode, Uri};
use js_sys::{Array, JsString, Object, Uint8Array};
use omegaupload_common::{Expiration, PartialParsedUrl};
use reqwasm::http::Request;
use wasm_bindgen::prelude::{wasm_bindgen, Closure};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::{spawn_local, JsFuture};
use web_sys::{window, Event, IdbDatabase, IdbObjectStore, IdbOpenDbRequest, IdbTransactionMode};

use crate::decrypt::decrypt;

mod decrypt;

#[wasm_bindgen]
extern "C" {
    fn loadFromDb();
    fn createNotFoundUi();
}

fn main() {
    let window = window().unwrap();
    let url = String::from(window.location().to_string());
    let request_uri = {
        let mut uri_parts = url.parse::<Uri>().unwrap().into_parts();
        if let Some(parts) = uri_parts.path_and_query.as_mut() {
            *parts = PathAndQuery::from_str(&format!("/api{}", parts.path())).unwrap();
        }
        Uri::from_parts(uri_parts).unwrap()
    };

    if window.location().pathname().unwrap() == "/" {
    } else {
        spawn_local(a(request_uri, url));
    }
}

#[allow(clippy::future_not_send)]
async fn a(request_uri: Uri, url: String) {
    match Request::get(&request_uri.to_string()).send().await {
        Ok(resp) if resp.status() == StatusCode::OK => {
            let expires = resp
                .headers()
                .get(EXPIRES.as_str())
                .ok()
                .flatten()
                .as_deref()
                .and_then(|v| Expiration::try_from(v).ok())
                .as_ref()
                .map_or_else(
                    || "This item does not expire.".to_string(),
                    Expiration::to_string,
                );

            let data = {
                Uint8Array::new(
                    &JsFuture::from(resp.as_raw().array_buffer().unwrap())
                        .await
                        .unwrap(),
                )
                .to_vec()
            };

            let info = url
                .split_once('#')
                .map(|(_, fragment)| PartialParsedUrl::from(fragment))
                .unwrap_or_default();
            let key = info.decryption_key.unwrap();
            let nonce = info.nonce.unwrap();

            let result = decrypt(data, key, nonce, None);

            let decrypted = match result {
                Ok(decrypted) => decrypted,
                Err(err) => {
                    // log!("decryption error: {}", err);
                    // return Box::new(PasteError(err));
                    unimplemented!()
                }
            };

            let db_open_req = window()
                .unwrap()
                .indexed_db()
                .unwrap()
                .unwrap()
                .open("omegaupload")
                .unwrap();

            // On success callback
            let on_success = Closure::once(Box::new(move |event: Event| {
                let target: IdbOpenDbRequest = event.target().unwrap().dyn_into().unwrap();
                let db: IdbDatabase = target.result().unwrap().dyn_into().unwrap();
                let transaction: IdbObjectStore = db
                    .transaction_with_str_and_mode("decrypted data", IdbTransactionMode::Readwrite)
                    .unwrap()
                    .object_store("decrypted data")
                    .unwrap();

                let decrypted_object = Array::new();
                match &decrypted {
                    DecryptedData::String(s) => {
                        let entry = Array::new();
                        entry.push(&JsString::from("data"));
                        entry.push(&JsValue::from_str(s));
                        decrypted_object.push(&entry);

                        let entry = Array::new();
                        entry.push(&JsString::from("type"));
                        entry.push(&JsString::from("string"));
                        decrypted_object.push(&entry);

                        let entry = Array::new();
                        entry.push(&JsString::from("expiration"));
                        entry.push(&JsString::from(expires.to_string()));
                        decrypted_object.push(&entry);
                    }
                    DecryptedData::Blob(blob) => {
                        let entry = Array::new();
                        entry.push(&JsString::from("data"));
                        entry.push(blob);
                        decrypted_object.push(&entry);

                        let entry = Array::new();
                        entry.push(&JsString::from("type"));
                        entry.push(&JsString::from("blob"));
                        decrypted_object.push(&entry);

                        let entry = Array::new();
                        entry.push(&JsString::from("expiration"));
                        entry.push(&JsString::from(expires.to_string()));
                        decrypted_object.push(&entry);
                    }
                    DecryptedData::Image(blob, (width, height), size) => {
                        let entry = Array::new();
                        entry.push(&JsString::from("data"));
                        entry.push(blob);
                        decrypted_object.push(&entry);

                        let entry = Array::new();
                        entry.push(&JsString::from("type"));
                        entry.push(&JsString::from("image"));
                        decrypted_object.push(&entry);

                        let entry = Array::new();
                        entry.push(&JsString::from("width"));
                        entry.push(&JsValue::from(*width));
                        decrypted_object.push(&entry);

                        let entry = Array::new();
                        entry.push(&JsString::from("height"));
                        entry.push(&JsValue::from(*height));
                        decrypted_object.push(&entry);

                        let entry = Array::new();
                        entry.push(&JsString::from("button"));
                        entry.push(&JsString::from(format!(
                            "Download {} \u{2014} {} by {}",
                            Byte::from_bytes(*size as u128).get_appropriate_unit(true),
                            width,
                            height,
                        )));
                        decrypted_object.push(&entry);

                        let entry = Array::new();
                        entry.push(&JsString::from("expiration"));
                        entry.push(&JsString::from(expires.to_string()));
                        decrypted_object.push(&entry);
                    }
                    DecryptedData::Audio(blob) => {
                        let entry = Array::new();
                        entry.push(&JsString::from("data"));
                        entry.push(blob);
                        decrypted_object.push(&entry);

                        let entry = Array::new();
                        entry.push(&JsString::from("type"));
                        entry.push(&JsString::from("audio"));
                        decrypted_object.push(&entry);

                        let entry = Array::new();
                        entry.push(&JsString::from("expiration"));
                        entry.push(&JsString::from(expires.to_string()));
                        decrypted_object.push(&entry);
                    }
                    DecryptedData::Video(blob) => {
                        let entry = Array::new();
                        entry.push(&JsString::from("data"));
                        entry.push(blob);
                        decrypted_object.push(&entry);

                        let entry = Array::new();
                        entry.push(&JsString::from("type"));
                        entry.push(&JsString::from("video"));
                        decrypted_object.push(&entry);

                        let entry = Array::new();
                        entry.push(&JsString::from("expiration"));
                        entry.push(&JsString::from(expires.to_string()));
                        decrypted_object.push(&entry);
                    }
                }

                let db_entry = Object::from_entries(&decrypted_object).unwrap();
                transaction
                    .put_with_key(
                        &db_entry,
                        &JsString::from(window().unwrap().location().pathname().unwrap()),
                    )
                    .unwrap()
                    .set_onsuccess(Some(
                        Closure::once(Box::new(|| {
                            log!("success");
                            loadFromDb();
                        }) as Box<dyn FnOnce()>)
                        .into_js_value()
                        .dyn_ref()
                        .unwrap(),
                    ));
            }) as Box<dyn FnOnce(Event)>);

            db_open_req.set_onsuccess(Some(on_success.into_js_value().dyn_ref().unwrap()));

            // On upgrade callback
            let on_upgrade = Closure::wrap(Box::new(move |event: Event| {
                let target: IdbOpenDbRequest = event.target().unwrap().dyn_into().unwrap();
                let db: IdbDatabase = target.result().unwrap().dyn_into().unwrap();
                let _obj_store = db.create_object_store("decrypted data").unwrap();
            }) as Box<dyn FnMut(Event)>);

            db_open_req.set_onupgradeneeded(Some(on_upgrade.into_js_value().dyn_ref().unwrap()));
        }
        Ok(resp) if resp.status() == StatusCode::NOT_FOUND => {
            createNotFoundUi();
        }
        Ok(resp) if resp.status() == StatusCode::BAD_REQUEST => {}
        Ok(err) => {}
        Err(err) => {}
    };
}
