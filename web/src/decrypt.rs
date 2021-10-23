use std::{collections::HashSet, sync::Arc};

use gloo_console::log;
use image::GenericImageView;
use js_sys::{Array, Uint8Array};
use omegaupload_common::{
    crypto::{open_in_place, Key, Nonce},
    Expiration,
};
use wasm_bindgen::JsCast;
use web_sys::Blob;
use yew::worker::{Agent, AgentLink, Context, HandlerId};
use yew::{html::Scope, worker::Public};

use crate::{DecryptedData, Paste, PasteCompleteConstructionError};

#[derive(Clone)]
pub struct DecryptionAgent {
    link: AgentLink<Self>,
}

impl Agent for DecryptionAgent {
    type Reach = Public<Self>;

    type Message = ();

    type Input = DecryptionAgentMessage;

    type Output = Result<(DecryptedData, PasteContext), PasteCompleteConstructionError>;

    fn create(link: AgentLink<Self>) -> Self {
        Self { link }
    }

    fn update(&mut self, _: Self::Message) {}

    fn handle_input(
        &mut self,
        DecryptionAgentMessage { context, params }: Self::Input,
        id: HandlerId,
    ) {
        let DecryptionParams {
            data,
            key,
            nonce,
            maybe_password,
        } = params;

        self.link.respond(
            id,
            decrypt(data, key, nonce, maybe_password).map(|res| (res, context)),
        )
    }
}

pub struct DecryptionAgentMessage {
    context: PasteContext,
    params: DecryptionParams,
}

impl DecryptionAgentMessage {
    pub fn new(context: PasteContext, params: DecryptionParams) -> Self {
        Self { context, params }
    }
}

pub struct PasteContext {
    pub link: Scope<Paste>,
    pub expires: Option<Expiration>,
}

impl PasteContext {
    pub fn new(link: Scope<Paste>, expires: Option<Expiration>) -> Self {
        Self { link, expires }
    }
}

pub struct DecryptionParams {
    data: Vec<u8>,
    key: Key,
    nonce: Nonce,
    maybe_password: Option<Key>,
}

impl DecryptionParams {
    pub fn new(data: Vec<u8>, key: Key, nonce: Nonce, maybe_password: Option<Key>) -> Self {
        Self {
            data,
            key,
            nonce,
            maybe_password,
        }
    }
}

fn decrypt(
    mut container: Vec<u8>,
    key: Key,
    nonce: Nonce,
    maybe_password: Option<Key>,
) -> Result<DecryptedData, PasteCompleteConstructionError> {
    let container = &mut container;
    log!("stage 1 decryption start");
    if let Some(password) = maybe_password {
        open_in_place(container, &nonce.increment(), &password)
            .map_err(|_| PasteCompleteConstructionError::StageOneFailure)?;
    }

    log!("stage 2 decryption start");
    open_in_place(container, &nonce, &key)
        .map_err(|_| PasteCompleteConstructionError::StageTwoFailure)?;

    log!("stage 2 decryption end");
    if let Ok(decrypted) = std::str::from_utf8(&container) {
        Ok(DecryptedData::String(Arc::new(decrypted.to_owned())))
    } else {
        log!("blob conversion start");
        let blob_chunks = Array::new_with_length(container.chunks(65536).len().try_into().unwrap());
        for (i, chunk) in container.chunks(65536).enumerate() {
            let array = Uint8Array::new_with_length(chunk.len().try_into().unwrap());
            array.copy_from(&chunk);
            blob_chunks.set(i.try_into().unwrap(), array.dyn_into().unwrap());
        }
        let blob =
            Arc::new(Blob::new_with_u8_array_sequence(blob_chunks.dyn_ref().unwrap()).unwrap());
        log!("blob conversion end");

        if let Ok(image) = image::load_from_memory(&container) {
            Ok(DecryptedData::Image(
                blob,
                image.dimensions(),
                container.len(),
            ))
        } else {
            Ok(DecryptedData::Blob(blob))
        }
    }
}
