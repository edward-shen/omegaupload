use std::str::FromStr;

use anyhow::anyhow;
use http::uri::{Authority, PathAndQuery};
use omegaupload_common::crypto::{Key, Nonce};
use omegaupload_common::ParsedUrl;
use yew::format::{Binary, Nothing};
use yew::services::fetch::{FetchTask, Request, Response, StatusCode, Uri};
use yew::services::{ConsoleService, FetchService};
use yew::utils::window;
use yew::{html, Component, ComponentLink, Html, ShouldRender};
use yew_router::router::Router;
use yew_router::Switch;

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
    state: PasteState,
    // Need to keep this alive so that the fetch request doesn't get dropped
    _fetch_handle: FetchTask,
}

#[derive(Clone, PartialEq, Eq)]
enum PasteState {
    NotFound,
    Error,
    NeedInformation {
        data: Option<Vec<u8>>,
        key: Option<Key>,
        nonce: Option<Nonce>,
        needs_pw: bool,
    },
    Done {
        data: Vec<u8>,
        key: Key,
        nonce: Nonce,
        password: Option<Key>,
    },
}

impl PasteState {
    fn set_data(&mut self, new_data: Vec<u8>) {
        match self {
            PasteState::NeedInformation { data, .. } => {
                assert!(data.is_none());
                *data = Some(new_data);
            }
            _ => panic!("Tried to set data in invalid state"),
        }
    }

    fn set_key(&mut self, new_key: Key) {
        match self {
            PasteState::NeedInformation { key, .. } => {
                assert!(key.is_none());
                *key = Some(new_key);
            }
            _ => panic!("Tried to set key in invalid state"),
        }
    }

    fn set_nonce(&mut self, new_nonce: Nonce) {
        match self {
            PasteState::NeedInformation { nonce, .. } => {
                assert!(nonce.is_none());
                *nonce = Some(new_nonce);
            }
            _ => panic!("Tried to set key in invalid state"),
        }
    }

    fn is_completed(&self) -> bool {
        match self {
            PasteState::NeedInformation {
                data,
                key,
                nonce,
                needs_pw,
            } => todo!(),
            _ => panic!(),
        }
        assert!(matches!(self, PasteState::NeedInformation { .. }));

        true
    }
}

enum PasteMessage {
    Data(Vec<u8>),
    Error(anyhow::Error),
    DecryptionKey(Key),
    Nonce(Nonce),
    Password(Key),
    NotFound,
}

impl Component for Paste {
    type Message = PasteMessage;

    type Properties = ();

    fn create(_: Self::Properties, link: ComponentLink<Self>) -> Self {
        let url = String::from(window().location().to_string());
        let request_uri = {
            let mut uri_parts = url.parse::<Uri>().unwrap().into_parts();
            uri_parts
                .authority
                .as_mut()
                .map(|auth| *auth = Authority::from_str(auth.host()).unwrap());
            uri_parts.path_and_query.as_mut().map(|parts| {
                *parts = PathAndQuery::from_str(&format!("/api{}", parts.path())).unwrap()
            });
            Uri::from_parts(uri_parts).unwrap()
        };

        ConsoleService::log(&request_uri.to_string());

        let fetch = FetchService::fetch_binary(
            Request::get(request_uri).body(Nothing).unwrap(),
            link.callback(move |resp: Response<Binary>| match resp.status() {
                StatusCode::OK => PasteMessage::Data(resp.into_body().unwrap()),
                StatusCode::NOT_FOUND => PasteMessage::NotFound,
                code => PasteMessage::Error(anyhow!("Got resp error: {}", code)),
            }),
        );
        Self {
            state: PasteState::NeedInformation {
                data: None,
                key: None,
                nonce: None,
                needs_pw: false,
            },
            _fetch_handle: fetch.unwrap(),
        }
    }

    fn update(&mut self, msg: Self::Message) -> ShouldRender {
        match msg {
            PasteMessage::Data(data) => self.state.set_data(data),
            PasteMessage::Error(e) => self.state = PasteState::Error,
            PasteMessage::NotFound => self.state = PasteState::NotFound,
            PasteMessage::DecryptionKey(key) => self.state.set_key(key),
            PasteMessage::Nonce(nonce) => self.state.set_nonce(nonce),
            PasteMessage::Password(_) => todo!(),
        }
        true
    }

    fn change(&mut self, _props: Self::Properties) -> ShouldRender {
        false
    }

    fn view(&self) -> Html {
        match self.state {
            PasteState::NeedInformation { .. } => todo!(),
            PasteState::Done { .. } => {
                todo!()
            }
            PasteState::Error => html! {
                <main>
                    {"An error occurred. Please try again later."}
                </main>
            },
            PasteState::NotFound => html! {
                <main>
                    {"The paste you are looking for is not here."}
                </main>
            },
        }
    }
}
