use std::{hint::unreachable_unchecked, marker::PhantomData};

use js_sys::{Array, JsString, Object};
use wasm_bindgen::JsValue;

pub struct IdbObject<State>(Array, PhantomData<State>);

impl<State: IdbObjectState> IdbObject<State> {
    fn add_tuple<NextState>(self, key: &str, value: &JsValue) -> IdbObject<NextState> {
        let array = Array::new();
        array.push(&JsString::from(key));
        array.push(value);
        self.0.push(&array);
        IdbObject(self.0, PhantomData)
    }
}

impl From<IdbObject<Ready>> for Object {
    fn from(db_object: IdbObject<Ready>) -> Self {
        match Self::from_entries(db_object.as_ref()) {
            Ok(o) => o,
            // SAFETY: IdbObject maintains the invariant that it can eventually
            // be constructed into a JS object.
            _ => unsafe { unreachable_unchecked() },
        }
    }
}

impl IdbObject<NeedsType> {
    pub fn new() -> Self {
        Self(Array::new(), PhantomData)
    }

    pub fn archive(self) -> IdbObject<NeedsExpiration> {
        self.add_tuple("type", &JsString::from("archive"))
    }

    pub fn video(self) -> IdbObject<NeedsExpiration> {
        self.add_tuple("type", &JsString::from("video"))
    }

    pub fn audio(self) -> IdbObject<NeedsExpiration> {
        self.add_tuple("type", &JsString::from("audio"))
    }

    pub fn image(self) -> IdbObject<NeedsExpiration> {
        self.add_tuple("type", &JsString::from("image"))
    }

    pub fn blob(self) -> IdbObject<NeedsExpiration> {
        self.add_tuple("type", &JsString::from("blob"))
    }

    pub fn string(self) -> IdbObject<NeedsExpiration> {
        self.add_tuple("type", &JsString::from("string"))
    }
}

impl Default for IdbObject<NeedsType> {
    fn default() -> Self {
        Self::new()
    }
}

impl IdbObject<NeedsExpiration> {
    pub fn expiration_text(self, expires: &str) -> IdbObject<NeedsData> {
        self.add_tuple("expiration", &JsString::from(expires))
    }
}

impl IdbObject<NeedsData> {
    pub fn data(self, value: &JsValue) -> IdbObject<Ready> {
        self.add_tuple("data", value)
    }
}

impl IdbObject<Ready> {
    pub fn extra(self, key: &str, value: impl Into<JsValue>) -> Self {
        self.add_tuple(key, &value.into())
    }
}

impl AsRef<JsValue> for IdbObject<Ready> {
    fn as_ref(&self) -> &JsValue {
        self.0.as_ref()
    }
}

macro_rules! impl_idb_object_state {
    ($($ident:ident),*) => {
        pub trait IdbObjectState {}
        $(
            pub enum $ident {}
            impl IdbObjectState for $ident {}
        )*
    };
}

impl_idb_object_state!(NeedsType, NeedsExpiration, NeedsData, Ready);
