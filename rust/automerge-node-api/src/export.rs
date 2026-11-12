//! Building JS values from a document: materialisation, patches and patch
//! application.
//!
//! This follows `export_cache.rs` and the patch code in `interop.rs` of
//! `automerge-wasm` closely, so that the documents and patches the two
//! bindings produce are indistinguishable. Where Node-API allows something
//! cheaper than a call out of wasm, it is used: the hidden properties of an
//! object are defined in a single `napi_define_properties` call.

use std::collections::HashMap;
use std::sync::Arc;

use automerge as am;
use automerge::iter::{DocItem, DocObjItem, Span};
use automerge::{ChangeHash, ObjId, ObjType, Patch, PatchAction, Prop, ReadDoc, ROOT};
use automerge_js_common::Datatype;
use napi::bindgen_prelude::*;
use napi::{JsValue, Property, PropertyAttributes, ValueType};

use crate::error::{ApplyPatch, Export};
use crate::interop::{
    self, alloc, alloc_scalar, alloc_scalar_ref, datatype_name, export_path, unknown,
    ExternalTypes, RawValue,
};

const RAW_DATA_SYMBOL: &str = "_am_raw_value_";
const DATATYPE_SYMBOL: &str = "_am_datatype_";
const RAW_OBJECT_SYMBOL: &str = "_am_objectId";
const META_SYMBOL: &str = "_am_meta";

/// The maximum number of arguments passed to one `Array.prototype.splice`
/// call, see `sub_splice` in `automerge-wasm`.
const MAX_SPLICE_ARGS: usize = 32_000;

#[derive(Clone, Copy)]
pub(crate) struct CachedObject<'env> {
    pub(crate) inner: Object<'env>,
    pub(crate) outer: Object<'env>,
}

pub(crate) struct Exporter<'a, 'env> {
    env: &'env Env,
    doc: &'a am::AutoCommit,
    externals: &'a ExternalTypes,
    freeze: bool,
    raw_obj_sym: Unknown<'env>,
    datatype_sym: Unknown<'env>,
    meta_sym: Unknown<'env>,
    raw_data_sym: Unknown<'env>,
    datatypes: HashMap<Datatype, Unknown<'env>>,
    /// Objects already unwrapped while applying patches.
    pub(crate) objs: HashMap<ObjId, (ObjId, CachedObject<'env>)>,
    /// Objects being materialised, and for text objects the parent object
    /// and property to set the text on.
    obj_cache: HashMap<ObjId, (Object<'env>, Option<Prop>)>,
    to_freeze: Vec<Object<'env>>,
}

impl<'a, 'env> Exporter<'a, 'env> {
    pub(crate) fn new(
        env: &'env Env,
        doc: &'a am::AutoCommit,
        externals: &'a ExternalTypes,
        freeze: bool,
    ) -> Result<Self> {
        Ok(Self {
            env,
            doc,
            externals,
            freeze,
            raw_obj_sym: env.symbol_for(RAW_OBJECT_SYMBOL)?.to_unknown(),
            datatype_sym: env.symbol_for(DATATYPE_SYMBOL)?.to_unknown(),
            meta_sym: env.symbol_for(META_SYMBOL)?.to_unknown(),
            raw_data_sym: env.symbol_for(RAW_DATA_SYMBOL)?.to_unknown(),
            datatypes: HashMap::new(),
            objs: HashMap::new(),
            obj_cache: HashMap::new(),
            to_freeze: Vec::new(),
        })
    }

    fn datatype(&mut self, datatype: Datatype) -> Result<Unknown<'env>> {
        if let Some(v) = self.datatypes.get(&datatype) {
            return Ok(*v);
        }
        let v = datatype_name(self.env, datatype)?;
        self.datatypes.insert(datatype, v);
        Ok(v)
    }

    fn hidden(&self, key: Unknown<'env>, value: &Unknown<'_>) -> Result<Property> {
        Ok(Property::new()
            .with_name(self.env, key)?
            .with_value(value)
            .with_property_attributes(PropertyAttributes::Default))
    }

    fn set_value(
        &self,
        obj: &mut Object<'env>,
        key: Unknown<'env>,
        value: &Unknown<'_>,
    ) -> Result<()> {
        obj.define_properties(&[self.hidden(key, value)?])
    }

    pub(crate) fn set_raw_data(&self, obj: &mut Object<'env>, value: &Unknown<'_>) -> Result<()> {
        self.set_value(obj, self.raw_data_sym, value)
    }

    pub(crate) fn set_datatype(&self, obj: &mut Object<'env>, value: &Unknown<'_>) -> Result<()> {
        self.set_value(obj, self.datatype_sym, value)
    }

    pub(crate) fn set_meta(&self, obj: &mut Object<'env>, value: &Unknown<'_>) -> Result<()> {
        self.set_value(obj, self.meta_sym, value)
    }

    pub(crate) fn set_raw_object(&self, obj: &mut Object<'env>, value: &Unknown<'_>) -> Result<()> {
        self.set_value(obj, self.raw_obj_sym, value)
    }

    /// Define the object id, datatype and metadata of `obj` in one call.
    pub(crate) fn set_hidden(
        &self,
        obj: &mut Object<'env>,
        raw_obj: &Unknown<'_>,
        datatype: &Unknown<'_>,
        meta: &Unknown<'_>,
    ) -> Result<()> {
        obj.define_properties(&[
            self.hidden(self.raw_obj_sym, raw_obj)?,
            self.hidden(self.datatype_sym, datatype)?,
            self.hidden(self.meta_sym, meta)?,
        ])
    }

    fn get_hidden(&self, obj: &Object<'_>, key: Unknown<'env>) -> Result<Unknown<'env>> {
        let raw: RawValue = obj.get_property_unchecked(key)?;
        Ok(unknown(self.env, raw.0))
    }

    pub(crate) fn get_raw_data(&self, obj: &Object<'_>) -> Result<Unknown<'env>> {
        self.get_hidden(obj, self.raw_data_sym)
    }

    // -----------------------------------------------------------------------
    // Materialisation

    fn construct_object(
        &self,
        value: Object<'env>,
        datatype: Datatype,
    ) -> std::result::Result<Object<'env>, Export> {
        if let Some(constructor) = self.externals.get(&datatype) {
            let wrapped = constructor.construct(self.env, &value.to_unknown(), datatype)?;
            let mut wrapped = interop::as_object(&wrapped)?
                .ok_or_else(|| Export::InvalidDataHandler(datatype.to_string()))?;
            self.set_raw_data(&mut wrapped, &value.to_unknown())?;
            Ok(wrapped)
        } else {
            Ok(value)
        }
    }

    fn wrap_new_object(
        &mut self,
        value: Object<'env>,
        obj: &ObjId,
        datatype: Datatype,
        meta: &Unknown<'_>,
    ) -> std::result::Result<Object<'env>, Export> {
        let mut value = self.construct_object(value, datatype)?;
        let js_datatype = self.datatype(datatype)?;
        let js_objid = self.env.create_string(obj.to_string())?.to_unknown();
        self.set_hidden(&mut value, &js_objid, &js_datatype, meta)?;
        if self.freeze {
            self.to_freeze.push(value);
        }
        Ok(value)
    }

    fn make_map(
        &mut self,
        obj: ObjId,
        meta: &Unknown<'_>,
    ) -> std::result::Result<Object<'env>, Export> {
        let child = Object::new(self.env)?;
        self.obj_cache.insert(obj.clone(), (child, None));
        self.wrap_new_object(child, &obj, Datatype::Map, meta)
    }

    fn make_list(
        &mut self,
        obj: ObjId,
        datatype: Datatype,
        meta: &Unknown<'_>,
    ) -> std::result::Result<Object<'env>, Export> {
        let child = interop::empty_array(self.env)?;
        self.obj_cache.insert(obj.clone(), (child, None));
        self.wrap_new_object(child, &obj, datatype, meta)
    }

    fn wrap_scalar(
        &self,
        value: Unknown<'env>,
        datatype: Datatype,
    ) -> std::result::Result<Unknown<'env>, Export> {
        if let Some(constructor) = self.externals.get(&datatype) {
            let wrapped = constructor.construct(self.env, &value, datatype)?;
            let mut o = interop::as_object(&wrapped)?
                .ok_or_else(|| Export::InvalidDataHandler(datatype.to_string()))?;
            self.set_raw_data(&mut o, &value)?;
            let dt = datatype_name(self.env, datatype)?;
            self.set_datatype(&mut o, &dt)?;
            Ok(o.to_unknown())
        } else {
            Ok(value)
        }
    }

    fn make_value_ref(
        &mut self,
        parent: Object<'env>,
        prop: Prop,
        obj: ObjId,
        value: am::ValueRef<'_>,
        meta: &Unknown<'_>,
    ) -> std::result::Result<Unknown<'env>, Export> {
        Ok(match value {
            am::ValueRef::Object(ObjType::Map) => self.make_map(obj, meta)?.to_unknown(),
            am::ValueRef::Object(ObjType::Text) => {
                self.obj_cache.insert(obj, (parent, Some(prop)));
                self.env.create_string("")?.to_unknown()
            }
            am::ValueRef::Scalar(s) => {
                let (datatype, value) = alloc_scalar_ref(self.env, &s)?;
                self.wrap_scalar(value, datatype)?
            }
            am::ValueRef::Object(obj_type) => {
                self.make_list(obj, obj_type.into(), meta)?.to_unknown()
            }
        })
    }

    fn set_prop(obj: &mut Object<'env>, prop: &Prop, value: Unknown<'_>) -> Result<()> {
        match prop {
            Prop::Map(key) => obj.set_named_property(key, value),
            Prop::Seq(index) => obj.set_element(*index as u32, value),
        }
    }

    /// Materialise `obj` as a JS value, as `ExportCache::materialize` in
    /// `automerge-wasm`.
    pub(crate) fn materialize(
        &mut self,
        obj: ObjId,
        datatype: Datatype,
        heads: Option<&[ChangeHash]>,
        meta: &Unknown<'_>,
    ) -> std::result::Result<Unknown<'env>, Export> {
        if datatype == Datatype::Text {
            let text = match heads {
                Some(h) => self.doc.text_at(&obj, h)?,
                None => self.doc.text(&obj)?,
            };
            return Ok(self.env.create_string(&text)?.to_unknown());
        }
        let mut current_obj_id = Arc::new(obj.clone());
        let mut o = match datatype {
            Datatype::Map => self.make_map(obj.clone(), meta)?,
            Datatype::List => self.make_list(obj.clone(), datatype, meta)?,
            _ => return Err(Export::InvalidRoot),
        };
        let result = o.to_unknown();
        let mut index = 0;
        let mut buffer = String::new();
        let mut parent_prop: Option<Prop> = None;
        let doc = self.doc;
        for DocObjItem { obj, item } in doc.iter_at(obj, heads) {
            if obj != current_obj_id {
                if !buffer.is_empty() {
                    if let Some(p) = &parent_prop {
                        let s = self.env.create_string(&buffer)?.to_unknown();
                        Self::set_prop(&mut o, p, s)?;
                    }
                    buffer.truncate(0);
                }
                if let Some((new_o, new_p)) = self.obj_cache.get(obj.as_ref()) {
                    o = *new_o;
                    parent_prop = new_p.clone();
                } else {
                    continue; // disconnected object
                }
                current_obj_id = obj;
                index = 0;
            }
            match item {
                DocItem::Text(span) => {
                    buffer.push_str(span.as_str());
                }
                DocItem::Map(map) => {
                    let prop = Prop::Map(map.key.to_string());
                    let value = self.make_value_ref(o, prop.clone(), map.id(), map.value, meta)?;
                    Self::set_prop(&mut o, &prop, value)?;
                }
                DocItem::List(list) => {
                    let prop = Prop::Seq(index);
                    let value =
                        self.make_value_ref(o, prop.clone(), list.id(), list.value, meta)?;
                    Self::set_prop(&mut o, &prop, value)?;
                    index += 1;
                }
            }
        }
        if !buffer.is_empty() {
            if let Some(p) = &parent_prop {
                let s = self.env.create_string(&buffer)?.to_unknown();
                Self::set_prop(&mut o, p, s)?;
            }
        }
        for o in &mut self.to_freeze {
            o.freeze()?;
        }
        Ok(result)
    }

    // -----------------------------------------------------------------------
    // Values in patches and hydrate values

    /// A scalar (or an empty object) wrapped by a registered datatype, as
    /// `Automerge::export_value` in `automerge-wasm`.
    pub(crate) fn export_value(
        &self,
        (datatype, raw_value): (Datatype, Unknown<'env>),
    ) -> std::result::Result<Unknown<'env>, Export> {
        if let Some(function) = self.externals.get(&datatype) {
            let wrapped = function.construct(self.env, &raw_value, datatype)?;
            match wrapped.get_type()? {
                ValueType::Object | ValueType::Function => {
                    let mut o = interop::as_object(&wrapped)?.unwrap();
                    self.set_raw_data(&mut o, &raw_value)?;
                    let dt = datatype_name(self.env, datatype)?;
                    self.set_datatype(&mut o, &dt)?;
                    Ok(o.to_unknown())
                }
                ValueType::String => Ok(wrapped),
                _ => Err(Export::InvalidDataHandler(datatype.to_string())),
            }
        } else {
            Ok(raw_value)
        }
    }

    pub(crate) fn export_hydrate(
        &self,
        value: am::hydrate::Value,
    ) -> std::result::Result<Unknown<'env>, Export> {
        match value {
            am::hydrate::Value::Scalar(s) => self.export_value(alloc_scalar(self.env, &s)?),
            am::hydrate::Value::Map(h_map) => {
                let mut map = Object::new(self.env)?;
                for (k, v) in h_map.iter() {
                    let val = self.export_hydrate(v.value.clone())?;
                    map.set_named_property(k, val)?;
                }
                Ok(map.to_unknown())
            }
            am::hydrate::Value::List(h_list) => {
                let mut list = interop::empty_array(self.env)?;
                for (i, v) in h_list.iter().enumerate() {
                    let val = self.export_hydrate(v.value.clone())?;
                    list.set_element(i as u32, val)?;
                }
                Ok(list.to_unknown())
            }
            am::hydrate::Value::Text(text) => {
                Ok(self.env.create_string(text.to_string())?.to_unknown())
            }
        }
    }

    pub(crate) fn export_marks(
        &self,
        marks: &am::marks::MarkSet,
    ) -> std::result::Result<Object<'env>, Export> {
        let mut result = Object::new(self.env)?;
        for (name, value) in marks.iter() {
            let (_, v) = alloc(self.env, &value.into())?;
            result.set_named_property(name, v)?;
        }
        Ok(result)
    }

    pub(crate) fn export_span(&self, span: Span) -> std::result::Result<Object<'env>, Export> {
        let mut result = Object::new(self.env)?;
        match span {
            Span::Text { text, marks } => {
                result.set_named_property("type", "text")?;
                result.set_named_property("value", text)?;
                if let Some(m) = &marks {
                    if m.num_marks() > 0 {
                        result.set_named_property("marks", self.export_marks(m)?)?;
                    }
                }
            }
            Span::Block(b) => {
                result.set_named_property("type", "block")?;
                result.set_named_property("value", self.export_hydrate(b.into())?)?;
            }
        }
        Ok(result)
    }

    // -----------------------------------------------------------------------
    // Patches

    fn construct_patch_value(
        &self,
        value: &am::Value<'_>,
    ) -> std::result::Result<Unknown<'env>, Export> {
        let (datatype, value) = alloc(self.env, value)?;
        if let Some(external_type) = self.externals.get(&datatype) {
            external_type.construct(self.env, &value, datatype)
        } else {
            Ok(value)
        }
    }

    pub(crate) fn export_patches<I: IntoIterator<Item = Patch>>(
        &self,
        patches: I,
    ) -> std::result::Result<Array<'env>, Export> {
        let mut result = self.env.create_array(0)?;
        for (i, p) in patches.into_iter().enumerate() {
            result.set(i as u32, self.export_patch(p)?)?;
        }
        Ok(result)
    }

    fn export_patch(&self, p: Patch) -> std::result::Result<Object<'env>, Export> {
        let env = self.env;
        let mut result = Object::new(env)?;
        let path = p.path.as_slice();
        match p.action {
            PatchAction::PutMap {
                key,
                value,
                conflict,
                ..
            } => {
                result.set_named_property("action", "put")?;
                result
                    .set_named_property("path", export_path(env, path, Some(&Prop::Map(key)))?)?;
                result.set_named_property("value", self.construct_patch_value(&value.0)?)?;
                if conflict {
                    result.set_named_property("conflict", true)?;
                }
            }
            PatchAction::PutSeq {
                index,
                value,
                conflict,
                ..
            } => {
                result.set_named_property("action", "put")?;
                result
                    .set_named_property("path", export_path(env, path, Some(&Prop::Seq(index)))?)?;
                result.set_named_property("value", self.construct_patch_value(&value.0)?)?;
                if conflict {
                    result.set_named_property("conflict", true)?;
                }
            }
            PatchAction::Insert { index, values, .. } => {
                let conflicts = values.iter().map(|v| v.2).collect::<Vec<_>>();
                let mut js_values = env.create_array(0)?;
                for (i, v) in values.iter().enumerate() {
                    js_values.set(i as u32, self.construct_patch_value(&v.0)?)?;
                }
                result.set_named_property("action", "insert")?;
                result
                    .set_named_property("path", export_path(env, path, Some(&Prop::Seq(index)))?)?;
                result.set_named_property("values", js_values)?;
                if conflicts.iter().any(|c| *c) {
                    result.set_named_property("conflicts", conflicts)?;
                }
            }
            PatchAction::SpliceText {
                index,
                value,
                marks,
                ..
            } => {
                result.set_named_property("action", "splice")?;
                result
                    .set_named_property("path", export_path(env, path, Some(&Prop::Seq(index)))?)?;
                result.set_named_property("value", value.make_string())?;
                if let Some(m) = marks {
                    if m.num_marks() > 0 {
                        result.set_named_property("marks", self.export_marks(&m)?)?;
                    }
                }
            }
            PatchAction::Increment { prop, value, .. } => {
                result.set_named_property("action", "inc")?;
                result.set_named_property("path", export_path(env, path, Some(&prop))?)?;
                result.set_named_property("value", value as f64)?;
            }
            PatchAction::DeleteMap { key, .. } => {
                result.set_named_property("action", "del")?;
                result
                    .set_named_property("path", export_path(env, path, Some(&Prop::Map(key)))?)?;
            }
            PatchAction::DeleteSeq { index, length, .. } => {
                result.set_named_property("action", "del")?;
                result
                    .set_named_property("path", export_path(env, path, Some(&Prop::Seq(index)))?)?;
                if length > 1 {
                    result.set_named_property("length", length as f64)?;
                }
            }
            PatchAction::Mark { marks, .. } => {
                result.set_named_property("action", "mark")?;
                result.set_named_property("path", export_path(env, path, None)?)?;
                let mut marks_array = env.create_array(0)?;
                for (i, m) in marks.iter().enumerate() {
                    marks_array.set(i as u32, self.export_mark(m)?)?;
                }
                result.set_named_property("marks", marks_array)?;
            }
            PatchAction::Conflict { prop } => {
                result.set_named_property("action", "conflict")?;
                result.set_named_property("path", export_path(env, path, Some(&prop))?)?;
            }
        }
        Ok(result)
    }

    pub(crate) fn export_mark(
        &self,
        m: &am::marks::Mark,
    ) -> std::result::Result<Object<'env>, Export> {
        let mut mark = Object::new(self.env)?;
        let (_, value) = alloc(self.env, &m.value().into())?;
        mark.set_named_property("name", m.name())?;
        mark.set_named_property("value", value)?;
        mark.set_named_property("start", m.start as i32)?;
        mark.set_named_property("end", m.end as i32)?;
        Ok(mark)
    }

    // -----------------------------------------------------------------------
    // Applying patches

    fn global_function(&self, object: &str, name: &str) -> Result<Unknown<'env>> {
        let global = self.env.get_global()?;
        let obj: Object = global.get_named_property_unchecked(object)?;
        interop::get(self.env, &obj, name)
    }

    fn shallow_copy(&self, obj: &Object<'env>) -> Result<Object<'env>> {
        let copy = if obj.is_array()? {
            let from = self.global_function("Array", "from")?;
            let from: Function<RawValue, RawValue> = unsafe { from.cast() }?;
            from.call(RawValue(obj.raw()))?
        } else {
            let assign = self.global_function("Object", "assign")?;
            let assign: Function<FnArgs<(RawValue, RawValue)>, RawValue> =
                unsafe { assign.cast() }?;
            let target = Object::new(self.env)?;
            assign.call(FnArgs::from((RawValue(target.raw()), RawValue(obj.raw()))))?
        };
        unsafe { unknown(self.env, copy.0).cast() }
    }

    fn unwrap_object(
        &mut self,
        ext_val: &Object<'env>,
        meta: &Unknown<'_>,
    ) -> std::result::Result<(bool, ObjId, CachedObject<'env>), Export> {
        let id_val = self.get_hidden(ext_val, self.raw_obj_sym)?;
        let id = match interop::as_string(&id_val)? {
            None if id_val.get_type()? == ValueType::Undefined => ROOT,
            s => self.doc.import(&s.unwrap_or_default())?.0,
        };
        if let Some((id, result)) = self.objs.get(&id) {
            return Ok((true, id.clone(), *result));
        }
        let inner = self.get_raw_data(ext_val)?;
        let datatype_raw = self.get_hidden(ext_val, self.datatype_sym)?;
        let datatype = interop::as_string(&datatype_raw)?.and_then(|d| Datatype::parse(&d).ok());
        let inner = match inner.get_type()? {
            ValueType::Object | ValueType::Function => interop::as_object(&inner)?.unwrap(),
            _ => *ext_val,
        };
        let datatype = match datatype {
            Some(d) => d,
            None if inner.is_array()? => Datatype::List,
            None => Datatype::Map,
        };
        let inner = self.shallow_copy(&inner)?;
        let mut outer = self.construct_object(inner, datatype)?;
        self.set_hidden(&mut outer, &id_val, &datatype_raw, meta)?;
        let cached = CachedObject { inner, outer };
        self.objs.insert(id.clone(), (id.clone(), cached));
        Ok((false, id, cached))
    }

    /// Unwrap the root object so that the metadata of the result can be
    /// updated even if there are no patches.
    pub(crate) fn apply_root_meta(
        &mut self,
        object: Object<'env>,
        meta: &Unknown<'_>,
    ) -> std::result::Result<Object<'env>, ApplyPatch> {
        let (_, _, cached) = self.unwrap_object(&object, meta)?;
        Ok(cached.inner)
    }

    fn unwrap_scalar(&self, ext_val: Unknown<'env>) -> Result<Unknown<'env>> {
        if let Some(o) = interop::as_object(&ext_val)? {
            let inner = self.get_raw_data(&o)?;
            if inner.get_type()? != ValueType::Undefined {
                return Ok(inner);
            }
        }
        Ok(ext_val)
    }

    /// As `Automerge::wrap_object` in `automerge-wasm`.
    fn wrap_object(
        &self,
        value: Object<'env>,
        datatype: Datatype,
        id: &ObjId,
        meta: &Unknown<'_>,
    ) -> std::result::Result<Object<'env>, Export> {
        let mut value = self.construct_object(value, datatype)?;
        if matches!(datatype, Datatype::Map | Datatype::List) {
            let id = self.env.create_string(id.to_string())?.to_unknown();
            self.set_raw_object(&mut value, &id)?;
        }
        let dt = datatype_name(self.env, datatype)?;
        self.set_datatype(&mut value, &dt)?;
        self.set_meta(&mut value, meta)?;
        if self.freeze {
            value.freeze()?;
        }
        Ok(value)
    }

    fn maybe_wrap_object(
        &self,
        (datatype, raw_value): (Datatype, Unknown<'env>),
        id: &ObjId,
        meta: &Unknown<'_>,
    ) -> std::result::Result<Unknown<'env>, Export> {
        match raw_value.get_type()? {
            ValueType::Object | ValueType::Function => {
                let obj = interop::as_object(&raw_value)?.unwrap();
                Ok(self.wrap_object(obj, datatype, id, meta)?.to_unknown())
            }
            _ => self.export_value((datatype, raw_value)),
        }
    }

    fn sub_splice<'v, I: IntoIterator<Item = &'v (am::Value<'v>, ObjId, bool)>>(
        &self,
        array: &Object<'env>,
        index: usize,
        num_del: usize,
        values: I,
        meta: &Unknown<'_>,
    ) -> std::result::Result<(), ApplyPatch> {
        let splice = interop::get(self.env, array, "splice")?;
        let apply = self.global_function("Reflect", "apply")?;
        let apply: Function<FnArgs<(RawValue, RawValue, RawValue)>, RawValue> =
            unsafe { apply.cast() }?;
        let call = |args: Array<'env>| -> Result<()> {
            apply.call(FnArgs::from((
                RawValue(splice.raw()),
                RawValue(array.raw()),
                RawValue(args.raw()),
            )))?;
            Ok(())
        };
        if num_del > 0 {
            let mut args = self.env.create_array(2)?;
            args.set(0, index as u32)?;
            args.set(1, num_del as u32)?;
            call(args)?;
        }
        let wrapped = values
            .into_iter()
            .map(|v| self.maybe_wrap_object(alloc(self.env, &v.0)?, &v.1, meta))
            .collect::<std::result::Result<Vec<_>, _>>()?;
        for (n, chunk) in wrapped.chunks(MAX_SPLICE_ARGS).enumerate() {
            let mut args = self.env.create_array(0)?;
            args.set(0, (index + n * MAX_SPLICE_ARGS) as u32)?;
            args.set(1, 0u32)?;
            for (i, v) in chunk.iter().enumerate() {
                args.set(i as u32 + 2, *v)?;
            }
            call(args)?;
        }
        Ok(())
    }

    fn increment(
        &self,
        container: &mut Object<'env>,
        prop: &Prop,
        by: i64,
    ) -> std::result::Result<(), ApplyPatch> {
        let old = match prop {
            Prop::Map(key) => interop::get(self.env, container, key)?,
            Prop::Seq(i) => {
                let raw: RawValue = container.get_element(*i as u32)?;
                unknown(self.env, raw.0)
            }
        };
        let old = self.unwrap_scalar(old)?;
        let Some(old) = interop::as_f64(&old)? else {
            return Err(ApplyPatch::IncrementNonNumeric);
        };
        let new_value: am::Value<'_> = am::ScalarValue::counter(old as i64 + by).into();
        let new_value = self.export_value(alloc(self.env, &new_value)?)?;
        Self::set_prop(container, prop, new_value)?;
        Ok(())
    }

    fn apply_patch_to_array(
        &self,
        array: &mut Object<'env>,
        patch: &Patch,
        meta: &Unknown<'_>,
    ) -> std::result::Result<(), ApplyPatch> {
        match &patch.action {
            PatchAction::PutSeq { index, value, .. } => {
                let sub_val = self.maybe_wrap_object(alloc(self.env, &value.0)?, &value.1, meta)?;
                array.set_element(*index as u32, sub_val)?;
                Ok(())
            }
            PatchAction::DeleteSeq { index, length, .. } => {
                self.sub_splice(array, *index, *length, vec![], meta)
            }
            PatchAction::Insert { index, values, .. } => {
                self.sub_splice(array, *index, 0, values.iter(), meta)
            }
            PatchAction::Increment { prop, value, .. } => match prop {
                Prop::Seq(_) => self.increment(array, prop, *value),
                Prop::Map(_) => Err(ApplyPatch::IncrementKeyInSeq),
            },
            PatchAction::DeleteMap { .. } => Err(ApplyPatch::DeleteKeyFromSeq),
            PatchAction::PutMap { .. } => Err(ApplyPatch::PutKeyInSeq),
            PatchAction::SpliceText { .. } => Err(ApplyPatch::SpliceTextInSeq),
            PatchAction::Mark { .. } => Ok(()),
            PatchAction::Conflict { .. } => Ok(()),
        }
    }

    fn apply_patch_to_map(
        &self,
        map: &mut Object<'env>,
        patch: &Patch,
        meta: &Unknown<'_>,
    ) -> std::result::Result<(), ApplyPatch> {
        match &patch.action {
            PatchAction::PutMap { key, value, .. } => {
                let sub_val = self.maybe_wrap_object(alloc(self.env, &value.0)?, &value.1, meta)?;
                map.set_named_property(key, sub_val)?;
                Ok(())
            }
            PatchAction::DeleteMap { key, .. } => {
                map.delete_named_property(key)?;
                Ok(())
            }
            PatchAction::Increment { prop, value, .. } => match prop {
                Prop::Map(_) => self.increment(map, prop, *value),
                Prop::Seq(_) => Err(ApplyPatch::IncrementIndexInMap),
            },
            PatchAction::Conflict { .. } => Ok(()),
            PatchAction::Insert { .. } => Err(ApplyPatch::InsertInMap),
            PatchAction::DeleteSeq { .. } => Err(ApplyPatch::SpliceInMap),
            PatchAction::SpliceText { .. } => Err(ApplyPatch::SpliceTextInMap),
            PatchAction::PutSeq { .. } => Err(ApplyPatch::PutIdxInMap),
            PatchAction::Mark { .. } => Err(ApplyPatch::MarkInMap),
        }
    }

    fn apply_patch_to_text(
        &self,
        string: &Unknown<'env>,
        patch: &Patch,
    ) -> std::result::Result<Unknown<'env>, ApplyPatch> {
        let units = interop::utf16(string)?;
        let slice = |from: usize, to: usize| &units[from.min(units.len())..to.min(units.len())];
        let result = match &patch.action {
            PatchAction::DeleteSeq { index, length, .. } => {
                let mut r = slice(0, *index).to_vec();
                r.extend_from_slice(slice(index + length, units.len()));
                r
            }
            PatchAction::SpliceText { index, value, .. } => {
                let mut r = slice(0, *index).to_vec();
                r.extend(value.make_string().encode_utf16());
                r.extend_from_slice(slice(*index, units.len()));
                r
            }
            PatchAction::Insert { index, values, .. } => {
                let mut r = slice(0, *index).to_vec();
                r.extend("\u{fffc}".repeat(values.len()).encode_utf16());
                r.extend_from_slice(slice(*index, units.len()));
                r
            }
            _ => return Ok(*string),
        };
        Ok(self.env.create_string_utf16(&result)?.to_unknown())
    }

    pub(crate) fn apply_patch(
        &mut self,
        root: Object<'env>,
        patch: &Patch,
        meta: &Unknown<'_>,
    ) -> std::result::Result<Object<'env>, ApplyPatch> {
        let (_, root_id, root_cache) = self.unwrap_object(&root, meta)?;
        let mut current = root_cache;
        let mut current_id = root_id;
        for (i, p) in patch.path.iter().enumerate() {
            let prop = &p.1;
            let subval = match prop {
                Prop::Map(key) => interop::get(self.env, &current.inner, key)?,
                Prop::Seq(idx) => {
                    let raw: RawValue = current.inner.get_element(*idx as u32)?;
                    unknown(self.env, raw.0)
                }
            };
            if subval.get_type()? == ValueType::String && patch.path.len() - 1 == i {
                let new_text = self.apply_patch_to_text(&subval, patch)?;
                Self::set_prop(&mut current.inner, prop, new_text)?;
                return Ok(root_cache.outer);
            }
            if let Some(subval) = interop::as_object(&subval)? {
                let (cache_hit, id, cached_obj) = self.unwrap_object(&subval, meta)?;
                if !cache_hit {
                    Self::set_prop(&mut current.inner, prop, cached_obj.outer.to_unknown())?;
                }
                current = cached_obj;
                current_id = id;
            } else {
                return Ok(root); // invalid patch
            }
        }
        if current_id != patch.obj {
            return Ok(root);
        }
        if current.inner.is_array()? {
            self.apply_patch_to_array(&mut current.inner, patch, meta)?;
        } else {
            self.apply_patch_to_map(&mut current.inner, patch, meta)?;
        }
        Ok(root_cache.outer)
    }
}
