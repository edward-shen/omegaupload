#![warn(clippy::nursery, clippy::pedantic)]

use std::marker::PhantomData;
use std::str::FromStr;

use anyhow::{anyhow, Result};
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
        spawn_local(async {
            a(request_uri, url).await;
        });
    }
}

#[allow(clippy::future_not_send)]
async fn a(request_uri: Uri, url: String) -> Result<()> {
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
                let data_fut = resp
                    .as_raw()
                    .array_buffer()
                    .expect("Failed to get raw bytes from response");
                let data = JsFuture::from(data_fut)
                    .await
                    .expect("Failed to result array buffer future");
                Uint8Array::new(&data).to_vec()
            };

            let info = url
                .split_once('#')
                .map(|(_, fragment)| PartialParsedUrl::from(fragment))
                .unwrap_or_default();
            let key = info
                .decryption_key
                .expect("missing key should be handled in the future");
            let nonce = info.nonce.expect("missing nonce be handled in the future");

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

                let db_entry = Object::from_entries(decrypted_object.as_ref()).unwrap();
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
    }

    Ok(())
}

struct IdbObject<State>(Array, PhantomData<State>);

impl<State: IdbObjectState> IdbObject<State> {
    fn add_tuple<NextState>(self, key: &str, value: &JsValue) -> IdbObject<NextState> {
        let array = Array::new();
        array.push(&JsString::from(key));
        array.push(value);
        self.0.push(&array);
        IdbObject(self.0, PhantomData)
    }
}

impl IdbObject<NeedsType> {
    fn new() -> Self {
        Self(Array::new(), PhantomData)
    }

    fn video(self) -> IdbObject<NeedsExpiration> {
        self.add_tuple("type", &JsString::from("video"))
    }

    fn audio(self) -> IdbObject<NeedsExpiration> {
        self.add_tuple("type", &JsString::from("audio"))
    }

    fn image(self) -> IdbObject<NeedsExpiration> {
        self.add_tuple("type", &JsString::from("image"))
    }

    fn blob(self) -> IdbObject<NeedsExpiration> {
        self.add_tuple("type", &JsString::from("blob"))
    }

    fn string(self) -> IdbObject<NeedsExpiration> {
        self.add_tuple("type", &JsString::from("string"))
    }
}

impl IdbObject<NeedsExpiration> {
    fn expiration_text(self, expires: &str) -> IdbObject<NeedsData> {
        self.add_tuple("expiration", &JsString::from(expires))
    }
}

impl IdbObject<NeedsData> {
    fn data(self, value: &JsValue) -> IdbObject<Ready> {
        self.add_tuple("data", value)
    }
}

impl IdbObject<Ready> {
    fn extra(self, key: &str, value: impl Into<JsValue>) -> Self {
        self.add_tuple(key, &value.into())
    }
}

impl AsRef<JsValue> for IdbObject<Ready> {
    fn as_ref(&self) -> &JsValue {
        self.0.as_ref()
    }
}

trait IdbObjectState {}

enum NeedsType {}
impl IdbObjectState for NeedsType {}

enum NeedsExpiration {}
impl IdbObjectState for NeedsExpiration {}

enum NeedsData {}
impl IdbObjectState for NeedsData {}

enum Ready {}
impl IdbObjectState for Ready {}
