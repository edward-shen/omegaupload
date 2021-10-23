#![warn(clippy::nursery, clippy::pedantic)]

use std::fmt::{Debug, Display, Formatter};
use std::str::FromStr;
use std::sync::Arc;

use anyhow::{anyhow, bail, Context};
use byte_unit::Byte;
use bytes::Bytes;
use decrypt::DecryptionAgent;
use downcast_rs::{impl_downcast, Downcast};
use gloo_console::log;
use http::header::EXPIRES;
use http::uri::PathAndQuery;
use http::{StatusCode, Uri};
use image::GenericImageView;
use js_sys::{Array, ArrayBuffer, Uint8Array};
use omegaupload_common::crypto::{open, open_in_place, Key, Nonce};
use omegaupload_common::{Expiration, PartialParsedUrl};
use reqwasm::http::Request;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use web_sys::TextDecoder;
use web_sys::{Blob, Url};
use yew::agent::Dispatcher;
use yew::utils::window;
use yew::worker::Agent;
use yew::{html, Bridge, Bridged, Component, ComponentLink, Html, ShouldRender};
use yew::{Dispatched, Properties};
use yew_router::router::Router;
use yew_router::Switch;
use yewtil::future::LinkFuture;

use crate::decrypt::{DecryptionAgentMessage, DecryptionParams, PasteContext};

mod decrypt;

fn main() {
    yew::start_app::<App>();
}

struct App;
impl Component for App {
    type Message = ();
    type Properties = ();

    fn create(_: Self::Properties, _: ComponentLink<Self>) -> Self {
        Self
    }

    fn update(&mut self, _: Self::Message) -> ShouldRender {
        false
    }

    fn change(&mut self, _props: Self::Properties) -> ShouldRender {
        false
    }

    fn view(&self) -> Html {
        html! {
            <Router<Route> render={Router::render(render_route)} />
        }
    }
}

#[derive(Clone, Debug, Switch)]
enum Route {
    #[to = "/!"]
    Index,
    #[rest]
    Path(String),
}

#[allow(clippy::needless_pass_by_value)]
fn render_route(route: Route) -> Html {
    match route {
        Route::Index => html! {
            <main>
                <p>{ "Hello world" }</p>
            </main>
        },
        Route::Path(_) => html! {
            <main>
                <Paste/>
            </main>
        },
    }
}

pub struct Paste {
    state: Box<dyn PasteState>,
    _listener: Box<dyn Bridge<DecryptionAgent>>,
}

impl Component for Paste {
    type Message = Box<dyn PasteState>;

    type Properties = ();

    fn create(_: Self::Properties, link: ComponentLink<Self>) -> Self {
        let url = String::from(window().location().to_string());
        let request_uri = {
            let mut uri_parts = url.parse::<Uri>().unwrap().into_parts();
            if let Some(parts) = uri_parts.path_and_query.as_mut() {
                *parts = PathAndQuery::from_str(&format!("/api{}", parts.path())).unwrap();
            }
            Uri::from_parts(uri_parts).unwrap()
        };

        let handle_decryption_result = |res: <DecryptionAgent as Agent>::Output| {
            log!("Got decryption result back!");
            match res {
                Ok((decrypted, context)) => {
                    Box::new(PasteComplete::new(context.link, decrypted, context.expires))
                        as Box<dyn PasteState>
                }
                Err(e) => Box::new(PasteError(anyhow!("wtf"))) as Box<dyn PasteState>,
            }
        };

        let listener = DecryptionAgent::bridge(link.callback(handle_decryption_result));

        let link_clone = link.clone();
        link.send_future(async move {
            match Request::get(&request_uri.to_string()).send().await {
                Ok(resp) if resp.status() == StatusCode::OK => {
                    let expires = resp
                        .headers()
                        .get(EXPIRES.as_str())
                        .ok()
                        .flatten()
                        .as_deref()
                        .and_then(|v| Expiration::try_from(v).ok());

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

                    let mut decryption_agent = DecryptionAgent::dispatcher();

                    let params = DecryptionParams::new(data, key, nonce, None);
                    let ctx = PasteContext::new(link_clone, expires);
                    decryption_agent.send(DecryptionAgentMessage::new(ctx, params));
                    Box::new(PasteDecrypting(decryption_agent)) as Box<dyn PasteState>
                }
                Ok(resp) if resp.status() == StatusCode::NOT_FOUND => {
                    Box::new(PasteNotFound) as Box<dyn PasteState>
                }
                Ok(resp) if resp.status() == StatusCode::BAD_REQUEST => {
                    Box::new(PasteBadRequest) as Box<dyn PasteState>
                }
                Ok(err) => {
                    Box::new(PasteError(anyhow!("Got {}.", err.status()))) as Box<dyn PasteState>
                }
                Err(err) => Box::new(PasteError(anyhow!("Got {}.", err))) as Box<dyn PasteState>,
            }
        });

        Self {
            state: Box::new(PasteLoading),
            _listener: listener,
        }
    }

    fn update(&mut self, msg: Self::Message) -> ShouldRender {
        self.state = msg;
        true
    }

    fn change(&mut self, _props: Self::Properties) -> ShouldRender {
        false
    }

    fn view(&self) -> Html {
        if self.state.is::<PasteLoading>() {
            return html! {
                <p>{ "loading" }</p>
            };
        }

        if self.state.is::<PasteDecrypting>() {
            return html! {
                "decrypting"
            };
        }

        if self.state.is::<PasteNotFound>() {
            return html! {
                <section class={"hljs centered"}>
                    <p>{ "Either the paste has been burned or one never existed." }</p>
                </section>
            };
        }

        if self.state.is::<PasteBadRequest>() {
            return html! {
                <section class={"hljs centered"}>
                    <p>{ "Bad Request. Is this a valid paste URL?" }</p>
                </section>
            };
        }

        if let Some(error) = self.state.downcast_ref::<PasteError>() {
            return html! {
                <section class={"hljs centered"}><p>{ error.0.to_string() }</p></section>
            };
        }

        if let Some(partial_paste) = self.state.downcast_ref::<PastePartial>() {
            return partial_paste.view();
        }

        if let Some(paste) = self.state.downcast_ref::<PasteComplete>() {
            return paste.view();
        }

        html! {
            "An internal error occurred: client is in unknown state!"
        }
    }
}

struct PasteError(anyhow::Error);

#[derive(Debug)]
struct PastePartial {
    parent: ComponentLink<Paste>,
    dispatcher: Dispatcher<DecryptionAgent>,
    data: Bytes,
    expires: Option<Expiration>,
    key: Option<Key>,
    nonce: Option<Nonce>,
    password: Option<Key>,
    needs_pw: bool,
}

#[derive(Properties, Clone)]
struct PasteComplete {
    parent: ComponentLink<Paste>,
    decrypted: DecryptedData,
    expires: Option<Expiration>,
}

#[derive(Clone)]
pub enum DecryptedData {
    String(Arc<String>),
    Blob(Arc<Blob>),
    Image(Arc<Blob>, (u32, u32), usize),
}

pub trait PasteState: Downcast {}
impl_downcast!(PasteState);

impl PasteState for PasteError {}
impl PasteState for PastePartial {}
impl PasteState for PasteComplete {}

macro_rules! impl_paste_type_state {
    (
        $($state:ident),* $(,)?
    ) => {
        $(
            struct $state;
            impl PasteState for $state {}
        )*
    };
}

impl_paste_type_state!(PasteLoading, PasteNotFound, PasteBadRequest);

struct PasteDecrypting(Dispatcher<DecryptionAgent>);

impl PasteState for PasteDecrypting {}

impl PastePartial {
    fn new(
        data: Bytes,
        expires: Option<Expiration>,
        partial_parsed_url: &PartialParsedUrl,
        parent: ComponentLink<Paste>,
    ) -> Self {
        Self {
            parent,
            dispatcher: DecryptionAgent::dispatcher(),
            data,
            expires,
            key: partial_parsed_url.decryption_key,
            nonce: partial_parsed_url.nonce,
            password: None,
            needs_pw: partial_parsed_url.needs_password,
        }
    }
}

enum PartialPasteMessage {
    DecryptionKey(Key),
    Nonce(Nonce),
    Password(Key),
}

impl Component for PastePartial {
    type Message = PartialPasteMessage;

    type Properties = ();

    fn create(_: Self::Properties, _: ComponentLink<Self>) -> Self {
        unimplemented!()
    }

    fn update(&mut self, msg: Self::Message) -> ShouldRender {
        match msg {
            PartialPasteMessage::DecryptionKey(key) => self.key = Some(key),
            PartialPasteMessage::Nonce(nonce) => self.nonce = Some(nonce),
            PartialPasteMessage::Password(password) => self.password = Some(password),
        }

        match (self.key, self.nonce, self.password) {
            (Some(key), Some(nonce), maybe_password)
                if (self.needs_pw && maybe_password.is_some())
                    || (!self.needs_pw && maybe_password.is_none()) =>
            {
                let parent = self.parent.clone();
                let mut data = self.data.to_vec();
                let expires = self.expires;

                // self.dispatcher.send((data, key, nonce, maybe_password));
                todo!()
            }
            _ => (),
        }

        // parent should re-render so this element should be dropped; no point
        // in saying this needs to be re-rendered.
        false
    }

    fn change(&mut self, _props: Self::Properties) -> ShouldRender {
        false
    }

    fn view(&self) -> Html {
        html! {
            "got partial data"
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

impl PasteComplete {
    fn new(
        parent: ComponentLink<Paste>,
        decrypted: DecryptedData,
        expires: Option<Expiration>,
    ) -> Self {
        Self {
            parent,
            decrypted,
            expires,
        }
    }

    fn view(&self) -> Html {
        match &self.decrypted {
            DecryptedData::String(decrypted) => html! {
                    html! {
                        <>
                            <pre class="paste">
                                <header class="unselectable">
                                {
                                    self.expires.as_ref().map(ToString::to_string).unwrap_or_else(||
                                        "This paste will not expire.".to_string()
                                    )
                                }
                                </header>
                                <hr />
                                <code>{decrypted}</code>
                            </pre>

                            <script>
                            {"
                                hljs.highlightAll();
                                hljs.initLineNumbersOnLoad();
                            "}
                            </script>
                        </>
                    }
            },
            DecryptedData::Blob(decrypted) => {
                let object_url = Url::create_object_url_with_blob(decrypted);
                if let Ok(object_url) = object_url {
                    let file_name = window().location().pathname().unwrap_or("file".to_string());
                    let mut cloned = self.clone();
                    let decrypted_ref = Arc::clone(&decrypted);
                    let display_anyways_callback =
                        self.parent.callback_future_once(|_| async move {
                            let array_buffer: ArrayBuffer =
                                JsFuture::from(decrypted_ref.array_buffer())
                                    .await
                                    .unwrap()
                                    .dyn_into()
                                    .unwrap();
                            let decoder = TextDecoder::new().unwrap();
                            cloned.decrypted = decoder
                                .decode_with_buffer_source(&array_buffer)
                                .map(Arc::new)
                                .map(DecryptedData::String)
                                .unwrap();
                            Box::new(cloned) as Box<dyn PasteState>
                        });
                    html! {
                        <section class="hljs fullscreen centered">
                            <div class="centered">
                                <p>{ "Found a binary file." }</p>
                                <a href=object_url download=file_name class="hljs-meta">{"Download"}</a>
                            </div>
                            <p onclick=display_anyways_callback class="display-anyways hljs-meta">{ "Display anyways?" }</p>
                        </section>
                    }
                } else {
                    // This branch really shouldn't happen, but might as well
                    // try and give a user-friendly error message.
                    html! {
                        <section class="hljs centered">
                            <p>{ "Failed to create an object URL for the decrypted file. Try reloading the page?" }</p>
                        </section>
                    }
                }
            }
            DecryptedData::Image(decrypted, (width, height), size) => {
                let object_url = Url::create_object_url_with_blob(decrypted);
                if let Ok(object_url) = object_url {
                    let file_name = window().location().pathname().unwrap_or("file".to_string());
                    html! {
                        <section class="hljs fullscreen centered">
                            <img src=object_url.clone() />
                            <a href=object_url download=file_name class="hljs-meta">
                                {
                                    format!(
                                        "Download {} \u{2014} {} by {}",
                                        Byte::from_bytes(*size as u128).get_appropriate_unit(true),
                                        width, height,
                                    )
                                }
                            </a>
                        </section>
                    }
                } else {
                    // This branch really shouldn't happen, but might as well
                    // try and give a user-friendly error message.
                    html! {
                        <section class="hljs fullscreen centered">
                            <p>{ "Failed to create an object URL for the decrypted file. Try reloading the page?" }</p>
                        </section>
                    }
                }
            }
        }
    }
}
