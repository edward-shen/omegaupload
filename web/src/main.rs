#![warn(clippy::nursery, clippy::pedantic)]

use std::fmt::Debug;
use std::str::FromStr;

use anyhow::{anyhow, bail};
use bytes::Bytes;
use downcast_rs::{impl_downcast, Downcast};
use http::header::EXPIRES;
use http::uri::PathAndQuery;
use http::{StatusCode, Uri};
use omegaupload_common::crypto::{open, Key, Nonce};
use omegaupload_common::{Expiration, PartialParsedUrl};
use yew::format::Nothing;
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
                    let partial = match resp.bytes().await {
                        Ok(bytes) => PastePartial::new(
                            bytes,
                            expires,
                            &url.split_once('#')
                                .map(|(_, fragment)| PartialParsedUrl::from(fragment))
                                .unwrap_or_default(),
                            link_clone,
                        ),
                        Err(e) => {
                            return Box::new(PasteError(anyhow!("Got {}.", e)))
                                as Box<dyn PasteState>
                        }
                    };

                    if let Ok(completed) = PasteComplete::try_from(partial.clone()) {
                        Box::new(completed) as Box<dyn PasteState>
                    } else {
                        Box::new(partial) as Box<dyn PasteState>
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
                <section class={"hljs error"}>
                    <p>{ "Either the paste has been burned or one never existed." }</p>
                </section>
            };
        }

        if self.state.is::<PasteBadRequest>() {
            return html! {
                <section class={"hljs error"}>
                    <p>{ "Bad Request. Is this a valid paste URL?" }</p>
                </section>
            };
        }

        if let Some(error) = self.state.downcast_ref::<PasteError>() {
            return html! {
                <section class={"hljs error"}><p>{ error.0.to_string() }</p></section>
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
    data: Bytes,
    expires: Option<Expiration>,
    key: Key,
    nonce: Nonce,
    password: Option<Key>,
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
                let data = self.data.clone();
                let expires = self.expires;
                self.parent.callback_once(move |Nothing| {
                    Box::new(PasteComplete::new(
                        data,
                        expires,
                        key,
                        nonce,
                        maybe_password,
                    )) as Box<dyn PasteState>
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

impl TryFrom<PastePartial> for PasteComplete {
    type Error = anyhow::Error;

    fn try_from(partial: PastePartial) -> Result<Self, Self::Error> {
        match partial {
            PastePartial {
                data,
                key: Some(key),
                expires,
                nonce: Some(nonce),
                password: Some(password),
                needs_pw: true,
                ..
            } => Ok(Self {
                data,
                expires,
                key,
                nonce,
                password: Some(password),
            }),
            PastePartial {
                data,
                key: Some(key),
                expires,
                nonce: Some(nonce),
                needs_pw: false,
                ..
            } => Ok(Self {
                data,
                key,
                expires,
                nonce,
                password: None,
            }),
            _ => bail!("missing field"),
        }
    }
}

impl PasteComplete {
    fn new(
        data: Bytes,
        expires: Option<Expiration>,
        key: Key,
        nonce: Nonce,
        password: Option<Key>,
    ) -> Self {
        Self {
            data,
            expires,
            key,
            nonce,
            password,
        }
    }

    fn view(&self) -> Html {
        let stage_one = self.password.map_or_else(
            || self.data.to_vec(),
            |password| open(&self.data, &self.nonce.increment(), &password).unwrap(),
        );
        let decrypted = open(&stage_one, &self.nonce, &self.key).unwrap();

        if let Ok(str) = String::from_utf8(decrypted) {
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
                    <code>{str}</code>
                </pre>

                <script>{"
                    hljs.highlightAll();
                    hljs.initLineNumbersOnLoad();
                "}</script>
                </>
            }
        } else {
            html! { "binary" }
        }
    }
}
