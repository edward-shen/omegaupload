use std::convert::TryFrom;
use std::fmt::Debug;
use std::str::FromStr;

use anyhow::{anyhow, bail};
use bytes::Bytes;
use downcast_rs::{impl_downcast, Downcast};
use http::uri::PathAndQuery;
use http::{StatusCode, Uri};
use omegaupload_common::crypto::{open, Key, Nonce};
use omegaupload_common::PartialParsedUrl;
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
            uri_parts.path_and_query.as_mut().map(|parts| {
                *parts = PathAndQuery::from_str(&format!("/api{}", parts.path())).unwrap()
            });
            Uri::from_parts(uri_parts).unwrap()
        };

        let link_clone = link.clone();
        link.send_future(async move {
            match reqwest::get(&request_uri.to_string()).await {
                Ok(resp) if resp.status() == StatusCode::OK => {
                    let partial = match resp.bytes().await {
                        Ok(bytes) => PastePartial::new(
                            bytes,
                            url.split_once('#')
                                .map(|(_, fragment)| PartialParsedUrl::from(fragment))
                                .unwrap_or_default(),
                            link_clone,
                        ),
                        Err(e) => {
                            return Box::new(PasteError(anyhow!("Got resp error: {}", e)))
                                as Box<dyn PasteState>
                        }
                    };

                    if let Ok(completed) = PasteComplete::try_from(partial.clone()) {
                        Box::new(completed) as Box<dyn PasteState>
                    } else {
                        Box::new(partial) as Box<dyn PasteState>
                    }
                }
                Ok(err) => Box::new(PasteError(anyhow!("Got resp error: {}", err.status())))
                    as Box<dyn PasteState>,
                Err(err) => {
                    Box::new(PasteError(anyhow!("Got resp error: {}", err))) as Box<dyn PasteState>
                }
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
                <p>{ "Either the paste has been burned or one never existed." }</p>
            };
        }

        if let Some(error) = self.state.downcast_ref::<PasteError>() {
            return html! {
                <p>{ error.0.to_string() }</p>
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

struct PasteLoading;
struct PasteNotFound;

struct PasteError(anyhow::Error);

#[derive(Properties, Clone, Debug)]
struct PastePartial {
    parent: ComponentLink<Paste>,
    data: Option<Bytes>,
    key: Option<Key>,
    nonce: Option<Nonce>,
    password: Option<Key>,
    needs_pw: bool,
}

#[derive(Properties, Clone)]
struct PasteComplete {
    data: Bytes,
    key: Key,
    nonce: Nonce,
    password: Option<Key>,
}

trait PasteState: Downcast {}
impl_downcast!(PasteState);
impl PasteState for PasteLoading {}
impl PasteState for PasteNotFound {}
impl PasteState for PasteError {}
impl PasteState for PastePartial {}
impl PasteState for PasteComplete {}

impl PastePartial {
    fn new(
        resp: Bytes,
        partial_parsed_url: PartialParsedUrl,
        parent: ComponentLink<Paste>,
    ) -> Self {
        Self {
            parent,
            data: Some(resp),
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

        match (self.data.clone(), self.key, self.nonce, self.password) {
            (Some(data), Some(key), Some(nonce), Some(password)) if self.needs_pw => {
                self.parent.callback(move |Nothing| {
                    Box::new(PasteComplete::new(data.clone(), key, nonce, Some(password)))
                        as Box<dyn PasteState>
                });
            }
            (Some(data), Some(key), Some(nonce), None) if !self.needs_pw => {
                self.parent.callback(move |Nothing| {
                    Box::new(PasteComplete::new(data.clone(), key, nonce, None))
                        as Box<dyn PasteState>
                });
            }
            _ => (),
        }

        // parent should re-render so this element should be dropped.
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
                data: Some(data),
                key: Some(key),
                nonce: Some(nonce),
                password: Some(password),
                needs_pw: true,
                ..
            } => Ok(PasteComplete {
                data,
                key,
                nonce,
                password: Some(password),
            }),
            PastePartial {
                data: Some(data),
                key: Some(key),
                nonce: Some(nonce),
                needs_pw: false,
                ..
            } => Ok(PasteComplete {
                data,
                key,
                nonce,
                password: None,
            }),
            _ => bail!("missing field"),
        }
    }
}

impl PasteComplete {
    fn new(data: Bytes, key: Key, nonce: Nonce, password: Option<Key>) -> Self {
        Self {
            data,
            key,
            nonce,
            password,
        }
    }

    fn view(&self) -> Html {
        let stage_one = if let Some(password) = self.password {
            open(&self.data, &self.nonce.increment(), &password).unwrap()
        } else {
            self.data.to_vec()
        };

        let decrypted = open(&stage_one, &self.nonce, &self.key).unwrap();

        if let Ok(str) = String::from_utf8(decrypted) {
            html! {
                <>
                <pre><code>{str}</code></pre>

                <script>{ "hljs.highlightAll();" }</script>
                </>
            }
        } else {
            html! { "binary" }
        }
    }
}
