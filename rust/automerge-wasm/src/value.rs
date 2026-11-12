//! Conversion of [`Datatype`]s to and from JS values. The datatypes
//! themselves are shared with the Node-API binding.

use wasm_bindgen::prelude::*;

pub(crate) use automerge_js_common::Datatype;

/// A datatype passed from JS was invalid.
#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub struct InvalidDatatype(#[from] automerge_js_common::datatype::InvalidDatatype);

impl From<InvalidDatatype> for JsValue {
    fn from(e: InvalidDatatype) -> Self {
        JsValue::from(e.to_string())
    }
}

pub(crate) fn datatype_from_js(datatype: JsValue) -> Result<Datatype, InvalidDatatype> {
    let datatype = datatype
        .as_string()
        .ok_or(automerge_js_common::datatype::InvalidDatatype::NotString)?;
    Ok(Datatype::parse(&datatype)?)
}

pub(crate) fn datatype_to_js(datatype: Datatype) -> JsValue {
    String::from(datatype).into()
}
