#![warn(clippy::nursery, clippy::pedantic)]

use std::fmt::{Debug, Display, Formatter};
use std::str::FromStr;

use anyhow::{anyhow, bail, Context};
use bytes::Bytes;
use downcast_rs::{impl_downcast, Downcast};
use gloo_console::log;
use http::header::EXPIRES;
use http::uri::PathAndQuery;
use http::{StatusCode, Uri};
use js_sys::{Array, ArrayBuffer, Uint8Array};
use omegaupload_common::crypto::{open, Key, Nonce};
use omegaupload_common::{Expiration, PartialParsedUrl};
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use web_sys::TextDecoder;
use web_sys::{Blob, Url};
use yew::utils::window;
use yew::Properties;
use yew::{html, Component, ComponentLink, Html, ShouldRender};
use yew_router::router::Router;
use yew_router::Switch;
use yewtil::future::LinkFuture;

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

struct Paste {
    state: Box<dyn PasteState>,
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

        let link_clone = link.clone();
        link.send_future(async move {
            match reqwest::get(&request_uri.to_string()).await {
                Ok(resp) if resp.status() == StatusCode::OK => {
                    let expires = resp
                        .headers()
                        .get(EXPIRES)
                        .and_then(|v| Expiration::try_from(v).ok());
                    let bytes = match resp.bytes().await {
                        Ok(bytes) => bytes,
                        Err(e) => {
                            return Box::new(PasteError(anyhow!("Got {}.", e)))
                                as Box<dyn PasteState>
                        }
                    };

                    let info = url
                        .split_once('#')
                        .map(|(_, fragment)| PartialParsedUrl::from(fragment))
                        .unwrap_or_default();
                    let key = info.decryption_key.unwrap();
                    let nonce = info.nonce.unwrap();

                    if let Ok(completed) = decrypt(bytes, key, nonce, None) {
                        Box::new(PasteComplete::new(link_clone, completed, expires))
                            as Box<dyn PasteState>
                    } else {
                        todo!()
                        // Box::new(partial) as Box<dyn PasteState>
                    }
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

#[derive(Properties, Clone, Debug)]
struct PastePartial {
    parent: ComponentLink<Paste>,
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
enum DecryptedData {
    String(String),
    Blob(Blob),
    Image(Blob),
}

trait PasteState: Downcast {}
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

impl PastePartial {
    fn new(
        data: Bytes,
        expires: Option<Expiration>,
        partial_parsed_url: &PartialParsedUrl,
        parent: ComponentLink<Paste>,
    ) -> Self {
        Self {
            parent,
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

    type Properties = Self;

    fn create(props: Self::Properties, _: ComponentLink<Self>) -> Self {
        props
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
                let data = self.data.clone();
                let expires = self.expires;

                self.parent.send_future(async move {
                    match decrypt(data, key, nonce, maybe_password) {
                        Ok(decrypted) => Box::new(PasteComplete::new(parent, decrypted, expires))
                            as Box<dyn PasteState>,
                        Err(e) => {
                            todo!()
                        }
                    }
                });
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

fn decrypt(
    encrypted: Bytes,
    key: Key,
    nonce: Nonce,
    maybe_password: Option<Key>,
) -> Result<DecryptedData, PasteCompleteConstructionError> {
    let stage_one = maybe_password.map_or_else(
        || Ok(encrypted.to_vec()),
        |password| open(&encrypted, &nonce.increment(), &password),
    );

    let stage_one = stage_one.map_err(|_| PasteCompleteConstructionError::StageOneFailure)?;

    let stage_two = open(&stage_one, &nonce, &key)
        .map_err(|_| PasteCompleteConstructionError::StageTwoFailure)?;

    if let Ok(decrypted) = std::str::from_utf8(&stage_two) {
        Ok(DecryptedData::String(decrypted.to_owned()))
    } else {
        let blob_chunks = Array::new_with_length(stage_two.chunks(65536).len().try_into().unwrap());
        for (i, chunk) in stage_two.chunks(65536).enumerate() {
            let array = Uint8Array::new_with_length(chunk.len().try_into().unwrap());
            array.copy_from(&chunk);
            blob_chunks.set(i.try_into().unwrap(), array.dyn_into().unwrap());
        }
        let blob = Blob::new_with_u8_array_sequence(blob_chunks.dyn_ref().unwrap()).unwrap();

        if image::guess_format(&stage_two).is_ok() {
            Ok(DecryptedData::Image(blob))
        } else {
            Ok(DecryptedData::Blob(blob))
        }
    }
}

#[derive(Debug)]
enum PasteCompleteConstructionError {
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
                        <pre class={"paste"}>
                            <header class={"hljs"}>
                            {
                                self.expires.as_ref().map(ToString::to_string).unwrap_or_else(||
                                    "This paste will not expire.".to_string()
                                )
                            }
                            </header>
                            <hr class={"hljs"} />
                            <code>{decrypted}</code>
                        </pre>

                        <script>{"
                            hljs.highlightAll();
                            hljs.initLineNumbersOnLoad();
                        "}</script>
                        </>
                    }
            },
            DecryptedData::Blob(decrypted) => {
                let object_url = Url::create_object_url_with_blob(decrypted);
                if let Ok(object_url) = object_url {
                    let file_name = window().location().pathname().unwrap_or("file".to_string());
                    let mut cloned = self.clone();
                    let decrypted_cloned = decrypted.clone();
                    let display_anyways_callback =
                        self.parent.callback_future_once(|_| async move {
                            let array_buffer: ArrayBuffer =
                                JsFuture::from(decrypted_cloned.array_buffer())
                                    .await
                                    .unwrap()
                                    .dyn_into()
                                    .unwrap();
                            let decoder = TextDecoder::new().unwrap();
                            cloned.decrypted = decoder
                                .decode_with_buffer_source(&array_buffer)
                                .map(DecryptedData::String)
                                .unwrap();
                            Box::new(cloned) as Box<dyn PasteState>
                        });
                    html! {
                        <section class="hljs centered">
                            <div class="centered">
                                <p>{ "Found a binary file." }</p>
                                <a href={object_url} download=file_name class="hljs-meta">{"Download"}</a>
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
            DecryptedData::Image(decrypted) => {
                let object_url = Url::create_object_url_with_blob(decrypted);
                if let Ok(object_url) = object_url {
                    let file_name = window().location().pathname().unwrap_or("file".to_string());
                    html! {
                        <section class="centered">
                            <img src={object_url.clone()} />
                            <a href={object_url} download=file_name class="hljs-meta">{"Download"}</a>
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
        }
    }
}
