//! Conversion between JS values and Automerge values.

use std::collections::HashMap;
use std::sync::Arc;

use automerge as am;
use automerge::{ChangeHash, ObjType, Prop, ScalarValue, ScalarValueRef};
use automerge_js_common as common;
use common::heads::BadChangeHashes;
use common::numbers::{int_is_safe, uint_is_safe};
use common::Datatype;
use napi::bindgen_prelude::*;
use napi::{sys, JsDate, JsString, JsValue, ValueType};

use crate::error::{ImportValue, InsertObject, JsValToHydrate};

/// A raw JS value passed to or returned from a stored function.
#[derive(Clone, Copy)]
pub(crate) struct RawValue(pub(crate) sys::napi_value);

impl ToNapiValue for RawValue {
    unsafe fn to_napi_value(_env: sys::napi_env, val: Self) -> Result<sys::napi_value> {
        Ok(val.0)
    }
}

impl FromNapiValue for RawValue {
    unsafe fn from_napi_value(_env: sys::napi_env, napi_val: sys::napi_value) -> Result<Self> {
        Ok(RawValue(napi_val))
    }
}

/// A new, empty array.
pub(crate) fn empty_array(env: &Env) -> Result<Object<'_>> {
    env.create_array(0)?.coerce_to_object()
}

pub(crate) fn unknown<'env>(env: &'env Env, raw: sys::napi_value) -> Unknown<'env> {
    // SAFETY: `raw` is a value which belongs to `env`.
    unsafe { Unknown::from_raw_unchecked(env.raw(), raw) }
}

/// The functions registered with `registerDatatype`.
pub(crate) struct ExternalType {
    construct: FunctionRef<RawValue, RawValue>,
    deconstruct: FunctionRef<RawValue, RawValue>,
}

impl ExternalType {
    pub(crate) fn new(
        construct: FunctionRef<RawValue, RawValue>,
        deconstruct: FunctionRef<RawValue, RawValue>,
    ) -> Self {
        Self {
            construct,
            deconstruct,
        }
    }

    pub(crate) fn construct<'env>(
        &self,
        env: &'env Env,
        value: &Unknown<'_>,
        datatype: Datatype,
    ) -> std::result::Result<Unknown<'env>, crate::error::Export> {
        let f = self.construct.borrow_back(env)?;
        let result = f
            .call(RawValue(value.raw()))
            .map_err(|e| crate::error::Export::CallDataHandler(datatype.to_string(), e))?;
        Ok(unknown(env, result.0))
    }

    pub(crate) fn deconstruct<'env>(
        &self,
        env: &'env Env,
        value: &Unknown<'_>,
    ) -> std::result::Result<Option<Unknown<'env>>, ImportValue> {
        let f = self.deconstruct.borrow_back(env)?;
        let result = f
            .call(RawValue(value.raw()))
            .map_err(ImportValue::CallDataHandler)?;
        let result = unknown(env, result.0);
        if result.get_type()? == ValueType::Undefined {
            Ok(None)
        } else {
            Ok(Some(result))
        }
    }
}

pub(crate) type ExternalTypes = HashMap<Datatype, Arc<ExternalType>>;

// ---------------------------------------------------------------------------
// Reading JS values

pub(crate) fn as_string(value: &Unknown<'_>) -> Result<Option<String>> {
    if value.get_type()? == ValueType::String {
        Ok(Some(unsafe { value.cast::<String>() }?))
    } else {
        Ok(None)
    }
}

pub(crate) fn as_f64(value: &Unknown<'_>) -> Result<Option<f64>> {
    if value.get_type()? == ValueType::Number {
        Ok(Some(unsafe { value.cast::<f64>() }?))
    } else {
        Ok(None)
    }
}

pub(crate) fn as_bool(value: &Unknown<'_>) -> Result<Option<bool>> {
    if value.get_type()? == ValueType::Boolean {
        Ok(Some(unsafe { value.cast::<bool>() }?))
    } else {
        Ok(None)
    }
}

pub(crate) fn as_date(value: &Unknown<'_>) -> Result<Option<f64>> {
    if value.get_type()? == ValueType::Object && value.is_date()? {
        Ok(Some(unsafe { value.cast::<JsDate>() }?.value_of()?))
    } else {
        Ok(None)
    }
}

/// The bytes of a `Uint8Array` (including a `Buffer`).
pub(crate) fn as_bytes(value: &Unknown<'_>) -> Result<Option<Vec<u8>>> {
    if value.get_type()? == ValueType::Object && value.is_typedarray()? {
        if let Ok(slice) = unsafe { value.cast::<Uint8ArraySlice>() } {
            return Ok(Some(slice.to_vec()));
        }
    }
    Ok(None)
}

pub(crate) fn as_object<'env>(value: &Unknown<'env>) -> Result<Option<Object<'env>>> {
    match value.get_type()? {
        ValueType::Object | ValueType::Function => Ok(Some(unsafe { value.cast::<Object>() }?)),
        _ => Ok(None),
    }
}

pub(crate) fn is_array(value: &Unknown<'_>) -> Result<bool> {
    Ok(value.get_type()? == ValueType::Object && value.is_array()?)
}

pub(crate) fn get<'env>(env: &'env Env, obj: &Object<'_>, key: &str) -> Result<Unknown<'env>> {
    let raw = obj.get_named_property_unchecked::<RawValue>(key)?;
    Ok(unknown(env, raw.0))
}

/// `Object.keys(obj)` paired with the values.
pub(crate) fn own_entries<'env>(
    env: &'env Env,
    obj: &Object<'_>,
) -> Result<Vec<(String, Unknown<'env>)>> {
    let keys = obj.get_all_property_names(
        KeyCollectionMode::OwnOnly,
        // napi-rs takes a single filter, so symbol keys are skipped below
        // rather than with `KeyFilter::SkipSymbols`.
        KeyFilter::Enumerable,
        KeyConversion::NumbersToStrings,
    )?;
    let len = keys.get_array_length()?;
    let mut result = Vec::with_capacity(len as usize);
    for i in 0..len {
        let key: RawValue = keys.get_element(i)?;
        let Some(key) = as_string(&unknown(env, key.0))? else {
            continue;
        };
        let value = get(env, obj, &key)?;
        result.push((key, value));
    }
    Ok(result)
}

pub(crate) fn array_elements<'env>(env: &'env Env, obj: &Object<'_>) -> Result<Vec<Unknown<'env>>> {
    let len = obj.get_array_length()?;
    let mut result = Vec::with_capacity(len as usize);
    for i in 0..len {
        let raw: RawValue = obj.get_element(i)?;
        result.push(unknown(env, raw.0));
    }
    Ok(result)
}

/// A human readable rendering of a value in error messages.
pub(crate) fn describe(value: &Unknown<'_>) -> String {
    let rendered = match value.get_type() {
        Ok(ValueType::Undefined) => "undefined".to_string(),
        Ok(ValueType::Null) => "null".to_string(),
        Ok(ValueType::Symbol) => "Symbol".to_string(),
        Ok(ValueType::Function) => "Function".to_string(),
        _ => value
            .coerce_to_string()
            .and_then(|s| s.into_utf8())
            .and_then(|s| s.into_owned())
            .unwrap_or_else(|_| "<value>".to_string()),
    };
    format!("JsValue({})", rendered)
}

// ---------------------------------------------------------------------------
// Scalars

fn bigint_to_i128(value: &Unknown<'_>) -> std::result::Result<i128, ImportValue> {
    let bigint = unsafe { value.cast::<BigInt>() }?;
    let (v, lossless) = bigint.get_i128();
    if lossless {
        Ok(v)
    } else if bigint.sign_bit {
        Ok(i128::MIN)
    } else {
        Ok(i128::MAX)
    }
}

fn bigint_to_i64(value: &Unknown<'_>) -> std::result::Result<i64, ImportValue> {
    let v = bigint_to_i128(value)?;
    if v > i64::MAX as i128 {
        return Err(ImportValue::BigIntTooLarge(
            format!("{}n", v),
            format!("{}n", i64::MAX),
        ));
    }
    if v < i64::MIN as i128 {
        return Err(ImportValue::BigIntTooSmall(
            format!("{}n", v),
            format!("{}n", i64::MIN),
        ));
    }
    Ok(v as i64)
}

fn bigint_to_u64(value: &Unknown<'_>) -> std::result::Result<u64, ImportValue> {
    let v = bigint_to_i128(value)?;
    if v > u64::MAX as i128 {
        return Err(ImportValue::BigIntTooLarge(
            format!("{}n", v),
            format!("{}n", u64::MAX),
        ));
    }
    if v < 0 {
        return Err(ImportValue::BigIntTooSmall(
            format!("{}n", v),
            "0n".to_string(),
        ));
    }
    Ok(v as u64)
}

fn bigint_to_scalar(value: &Unknown<'_>) -> std::result::Result<ScalarValue, ImportValue> {
    let v = bigint_to_i128(value)?;
    if v > i64::MAX as i128 {
        Ok(ScalarValue::Uint(bigint_to_u64(value)?))
    } else {
        Ok(ScalarValue::Int(bigint_to_i64(value)?))
    }
}

fn to_i64(value: &Unknown<'_>) -> std::result::Result<i64, ImportValue> {
    if value.get_type()? == ValueType::BigInt {
        bigint_to_i64(value)
    } else if let Some(v) = as_f64(value)? {
        Ok(v as i64)
    } else if let Some(d) = as_date(value)? {
        Ok(d as i64)
    } else {
        Err(ImportValue::Invalid(describe(value)))
    }
}

fn to_u64(value: &Unknown<'_>) -> std::result::Result<u64, ImportValue> {
    if value.get_type()? == ValueType::BigInt {
        bigint_to_u64(value)
    } else if let Some(v) = as_f64(value)? {
        Ok(v as u64)
    } else {
        Err(ImportValue::Invalid(describe(value)))
    }
}

/// Convert a JS value to a scalar, as `import_scalar` in `automerge-wasm`.
pub(crate) fn import_scalar(
    value: &Unknown<'_>,
    datatype: Option<Datatype>,
) -> std::result::Result<ScalarValue, ImportValue> {
    let result = match datatype {
        Some(Datatype::Boolean) => as_bool(value)?.map(ScalarValue::Boolean),
        Some(Datatype::Int) => Some(ScalarValue::Int(to_i64(value)?)),
        Some(Datatype::Uint) => Some(ScalarValue::Uint(to_u64(value)?)),
        Some(Datatype::Str) => as_string(value)?.map(|s| ScalarValue::Str(s.into())),
        Some(Datatype::F64) => as_f64(value)?.map(ScalarValue::F64),
        Some(Datatype::Bytes) => Some(ScalarValue::Bytes(
            as_bytes(value)?.ok_or_else(|| ImportValue::Invalid(describe(value)))?,
        )),
        Some(Datatype::Counter) => Some(ScalarValue::counter(to_i64(value)?)),
        Some(Datatype::Timestamp) => Some(ScalarValue::Timestamp(to_i64(value)?)),
        Some(Datatype::Null) => Some(ScalarValue::Null),
        Some(_) => return Err(ImportValue::ValueNotPrimitive),
        None => match value.get_type()? {
            ValueType::Null => Some(ScalarValue::Null),
            ValueType::Boolean => as_bool(value)?.map(ScalarValue::Boolean),
            ValueType::String => as_string(value)?.map(|s| ScalarValue::Str(s.into())),
            ValueType::BigInt => Some(bigint_to_scalar(value)?),
            ValueType::Number => as_f64(value)?.map(common::numbers::number_to_scalar),
            ValueType::Object => {
                if let Some(d) = as_date(value)? {
                    Some(ScalarValue::Timestamp(d as i64))
                } else {
                    as_bytes(value)?.map(ScalarValue::Bytes)
                }
            }
            _ => None,
        },
    };
    result.ok_or_else(|| ImportValue::Invalid(describe(value)))
}

pub(crate) fn date<'env>(env: &'env Env, ms: i64) -> Result<Unknown<'env>> {
    Ok(env.create_date(ms as f64)?.to_unknown())
}

pub(crate) fn bytes<'env>(env: &'env Env, bytes: &[u8]) -> Result<Unknown<'env>> {
    Uint8Array::from(bytes.to_vec()).into_unknown(env)
}

fn int<'env>(env: &'env Env, v: i64) -> Result<Unknown<'env>> {
    if int_is_safe(v) {
        (v as f64).into_unknown(env)
    } else {
        BigInt::from(v).into_unknown(env)
    }
}

fn uint<'env>(env: &'env Env, v: u64) -> Result<Unknown<'env>> {
    if uint_is_safe(v) {
        (v as f64).into_unknown(env)
    } else {
        BigInt::from(v).into_unknown(env)
    }
}

pub(crate) fn alloc_scalar<'env>(
    env: &'env Env,
    value: &ScalarValue,
) -> Result<(Datatype, Unknown<'env>)> {
    let datatype = Datatype::from(value);
    let value = match value {
        ScalarValue::Bytes(v) => bytes(env, v)?,
        ScalarValue::Str(v) => env.create_string(v)?.to_unknown(),
        ScalarValue::Int(v) => int(env, *v)?,
        ScalarValue::Uint(v) => uint(env, *v)?,
        ScalarValue::F64(v) => v.into_unknown(env)?,
        ScalarValue::Counter(v) => int(env, i64::from(v))?,
        ScalarValue::Timestamp(v) => date(env, *v)?,
        ScalarValue::Boolean(v) => v.into_unknown(env)?,
        ScalarValue::Null => Null.into_unknown(env)?,
        ScalarValue::Unknown { bytes: b, .. } => bytes(env, b)?,
    };
    Ok((datatype, value))
}

pub(crate) fn alloc_scalar_ref<'env>(
    env: &'env Env,
    value: &ScalarValueRef<'_>,
) -> Result<(Datatype, Unknown<'env>)> {
    let datatype = Datatype::from(value);
    let value = match value {
        ScalarValueRef::Bytes(v) => bytes(env, v)?,
        ScalarValueRef::Str(v) => env.create_string(v)?.to_unknown(),
        ScalarValueRef::Int(v) => int(env, *v)?,
        ScalarValueRef::Uint(v) => uint(env, *v)?,
        ScalarValueRef::F64(v) => v.into_unknown(env)?,
        ScalarValueRef::Counter(v) => int(env, *v)?,
        ScalarValueRef::Timestamp(v) => date(env, *v)?,
        ScalarValueRef::Boolean(v) => v.into_unknown(env)?,
        ScalarValueRef::Null => Null.into_unknown(env)?,
        ScalarValueRef::Unknown { bytes: b, .. } => bytes(env, b)?,
    };
    Ok((datatype, value))
}

/// A new, empty JS value for an Automerge value: an empty object, array or
/// string for objects, the value itself for scalars.
pub(crate) fn alloc<'env>(
    env: &'env Env,
    value: &am::Value<'_>,
) -> Result<(Datatype, Unknown<'env>)> {
    match value {
        am::Value::Object(o) => {
            let v = match o {
                ObjType::Map | ObjType::Table => Object::new(env)?.to_unknown(),
                ObjType::List => empty_array(env)?.to_unknown(),
                ObjType::Text => env.create_string("")?.to_unknown(),
            };
            Ok((Datatype::from(o), v))
        }
        am::Value::Scalar(s) => alloc_scalar(env, s.as_ref()),
    }
}

pub(crate) fn datatype_name<'env>(env: &'env Env, datatype: Datatype) -> Result<Unknown<'env>> {
    Ok(env.create_string(String::from(datatype))?.to_unknown())
}

// ---------------------------------------------------------------------------
// Heads, props, paths

pub(crate) fn parse_heads(
    heads: Option<Vec<String>>,
) -> std::result::Result<Option<Vec<ChangeHash>>, BadChangeHashes> {
    heads
        .map(|h| common::heads::parse_heads(h.into_iter().map(Some)))
        .transpose()
}

pub(crate) fn export_heads(heads: &[ChangeHash]) -> Vec<String> {
    heads.iter().map(|h| h.to_string()).collect()
}

pub(crate) fn to_prop(prop: Either<String, f64>) -> Prop {
    match prop {
        Either::A(s) => Prop::Map(s),
        Either::B(n) => common::obj::number_to_prop(n),
    }
}

pub(crate) fn prop_to_js<'env>(env: &'env Env, prop: &Prop) -> Result<Unknown<'env>> {
    match prop {
        Prop::Map(key) => Ok(env.create_string(key)?.to_unknown()),
        Prop::Seq(index) => (*index as f64).into_unknown(env),
    }
}

pub(crate) fn export_path<'env>(
    env: &'env Env,
    path: &[(am::ObjId, Prop)],
    end: Option<&Prop>,
) -> Result<Array<'env>> {
    let mut result = env.create_array(0)?;
    for (i, (_, p)) in path.iter().enumerate() {
        result.set(i as u32, prop_to_js(env, p)?)?;
    }
    if let Some(end) = end {
        result.set(path.len() as u32, prop_to_js(env, end)?)?;
    }
    Ok(result)
}

// ---------------------------------------------------------------------------
// Objects being inserted

pub(crate) enum JsObjType<'env> {
    Text(String),
    Map(Vec<(Prop, Unknown<'env>)>),
    List(Vec<(Prop, Unknown<'env>)>),
}

impl<'env> JsObjType<'env> {
    pub(crate) fn objtype(&self) -> ObjType {
        match self {
            Self::Text(_) => ObjType::Text,
            Self::Map(_) => ObjType::Map,
            Self::List(_) => ObjType::List,
        }
    }

    pub(crate) fn text(&self) -> Option<&str> {
        match self {
            Self::Text(s) => Some(s.as_ref()),
            _ => None,
        }
    }

    /// The values of this object; the characters of a text object.
    pub(crate) fn subvals(&self, env: &'env Env) -> Result<Vec<(Prop, Unknown<'env>)>> {
        match self {
            Self::Text(s) => s
                .chars()
                .enumerate()
                .map(|(i, c)| Ok((Prop::Seq(i), env.create_string(c.to_string())?.to_unknown())))
                .collect(),
            Self::Map(sub) | Self::List(sub) => Ok(sub.clone()),
        }
    }
}

fn list_entries<'env>(env: &'env Env, obj: &Object<'_>) -> Result<Vec<(Prop, Unknown<'env>)>> {
    Ok(array_elements(env, obj)?
        .into_iter()
        .enumerate()
        .map(|(i, v)| (Prop::Seq(i), v))
        .collect())
}

fn map_entries<'env>(env: &'env Env, obj: &Object<'_>) -> Result<Vec<(Prop, Unknown<'env>)>> {
    Ok(own_entries(env, obj)?
        .into_iter()
        .map(|(k, v)| (Prop::Map(k), v))
        .collect())
}

/// Read a JS value as the contents of a new object, as `import_obj` in
/// `automerge-wasm`.
pub(crate) fn import_obj<'env>(
    env: &'env Env,
    value: &Unknown<'_>,
    datatype: Option<Datatype>,
) -> std::result::Result<JsObjType<'env>, InsertObject> {
    match datatype {
        Some(Datatype::Map) => {
            let obj = as_object(value)?.ok_or(InsertObject::ValueNotObject)?;
            Ok(JsObjType::Map(map_entries(env, &obj)?))
        }
        Some(Datatype::List) => {
            if !is_array(value)? {
                return Err(InsertObject::ValueNotObject);
            }
            let obj = as_object(value)?.ok_or(InsertObject::ValueNotObject)?;
            Ok(JsObjType::List(list_entries(env, &obj)?))
        }
        Some(Datatype::Text) => {
            let text = as_string(value)?.ok_or(InsertObject::ValueNotObject)?;
            Ok(JsObjType::Text(text))
        }
        Some(_) => Err(InsertObject::ValueNotObject),
        None => {
            if is_array(value)? {
                let obj = as_object(value)?.ok_or(InsertObject::ValueNotObject)?;
                Ok(JsObjType::List(list_entries(env, &obj)?))
            } else if let Some(obj) = as_object(value)? {
                Ok(JsObjType::Map(map_entries(env, &obj)?))
            } else if let Some(s) = as_string(value)? {
                Ok(JsObjType::Text(s))
            } else {
                Err(InsertObject::ValueNotObject)
            }
        }
    }
}

/// Convert a JS value to a hydrate value, as `js_val_to_hydrate` in
/// `automerge-wasm`.
pub(crate) fn js_val_to_hydrate(
    env: &Env,
    externals: &ExternalTypes,
    js_val: &Unknown<'_>,
) -> std::result::Result<am::hydrate::Value, JsValToHydrate> {
    let mut found = None;
    for (dt, et) in externals.iter() {
        if let Some(v) = et.deconstruct(env, js_val)? {
            found = Some((*dt, v));
            break;
        }
    }
    let (datatype, value) = match found {
        Some((dt, v)) => (Some(dt), v),
        None => (None, *js_val),
    };
    if datatype.is_none() {
        if let Some(d) = as_date(&value)? {
            return Ok(am::hydrate::Value::Scalar(ScalarValue::Timestamp(d as i64)));
        }
        if let Some(b) = as_bytes(&value)? {
            return Ok(am::hydrate::Value::Scalar(ScalarValue::Bytes(b)));
        }
    }
    match import_obj(env, &value, datatype) {
        Ok(js_obj) => match js_obj.objtype() {
            ObjType::Map | ObjType::Table => {
                let mut obj = HashMap::new();
                for (p, v) in js_obj.subvals(env)? {
                    if let Prop::Map(key) = p {
                        obj.insert(key, js_val_to_hydrate(env, externals, &v)?);
                    }
                }
                Ok(am::hydrate::Value::Map(obj.into()))
            }
            ObjType::List => {
                let mut list = Vec::new();
                for (_, v) in js_obj.subvals(env)? {
                    list.push(js_val_to_hydrate(env, externals, &v)?);
                }
                Ok(am::hydrate::Value::List(list.into()))
            }
            ObjType::Text => {
                let text = js_obj.text().ok_or(JsValToHydrate::InvalidText)?;
                Ok(am::hydrate::Value::Text(am::hydrate::Text::new(
                    am::TextEncoding::Utf16CodeUnit,
                    text,
                )))
            }
        },
        Err(InsertObject::Napi(e)) => Err(e.into()),
        Err(_) => Ok(am::hydrate::Value::Scalar(import_scalar(&value, datatype)?)),
    }
}

/// UTF-16 code units of a JS string.
pub(crate) fn utf16(value: &Unknown<'_>) -> Result<Vec<u16>> {
    let s = unsafe { value.cast::<JsString>() }?;
    let len = s.utf16_len()?;
    let mut units = s.into_utf16()?.as_slice().to_vec();
    // the buffer napi-rs reads into may include a terminating NUL
    units.truncate(len);
    Ok(units)
}
