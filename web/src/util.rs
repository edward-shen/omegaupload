use wasm_bindgen::JsCast;
use web_sys::{Event, IdbDatabase, IdbOpenDbRequest};

/// # Panics
///
/// This will panic if event is not an event from the IDB API.
pub fn as_idb_db(event: &Event) -> IdbDatabase {
    let target: IdbOpenDbRequest = event.target().map(JsCast::unchecked_into).unwrap();
    target.result().map(JsCast::unchecked_into).unwrap()
}
