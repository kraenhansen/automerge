//! A Node-API binding to Automerge, built with napi-rs.
//!
//! The methods mirror the low-level API of `automerge-wasm` (see
//! `javascript/src/backend.ts` for the interface the JS package expects).
//! Parameters are typed with what napi-rs offers (`Option`, `Either`,
//! typed arrays) instead of untyped values, and errors are thrown as the
//! same JS class, with the same message, as `automerge-wasm` throws them.

use std::collections::HashMap;
use std::sync::Arc;

use automerge as am;
use automerge::marks::{ExpandMark, UpdateSpansConfig};
use automerge::transaction::{CommitOptions, Transactable};
use automerge::{
    AutoCommit, Change, ChangeHash, CursorPosition, ObjId, ObjType, OnPartialLoad, Prop, ReadDoc,
    ScalarValue, StringMigration, TextEncoding, Value, VerificationMode, ROOT,
};
use automerge_js_common as common;
use common::commit::Commit;
use common::error as cerr;
use common::fragment::FragmentLevelRange;
use common::heads::BadChangeHashes;
use common::obj::ImportObj;
use common::Datatype;
use napi::bindgen_prelude::*;
use napi::{Env, JsValue, ValueType};
use napi_derive::napi;

mod error;
mod export;
mod interop;
mod sync;

use error::*;
use export::Exporter;
use interop::{
    alloc, as_object, export_heads, import_obj, import_scalar, js_val_to_hydrate, parse_heads,
    to_prop, unknown, ExternalType, ExternalTypes, RawValue,
};
pub use sync::SyncState;

pub type Heads = Vec<String>;

#[napi]
pub struct Automerge {
    doc: AutoCommit,
    freeze: bool,
    external_types: ExternalTypes,
}

fn datatype(
    datatype: Option<String>,
) -> std::result::Result<Option<Datatype>, common::datatype::InvalidDatatype> {
    datatype.map(|d| Datatype::parse(&d)).transpose()
}

fn undefined(env: &Env) -> Result<Unknown<'_>> {
    ().into_unknown(env)
}

fn uint8arrays<T: AsRef<[u8]>>(items: impl IntoIterator<Item = T>) -> Vec<Uint8Array> {
    items
        .into_iter()
        .map(|c| Uint8Array::from(c.as_ref().to_vec()))
        .collect()
}

/// Call `JSON.parse` on a serialisation of `value`. This produces exactly the
/// values `serde_wasm_bindgen`'s JSON compatible serializer produces in
/// `automerge-wasm`.
fn to_json_value<'env, T: serde::Serialize>(env: &'env Env, value: &T) -> Result<Unknown<'env>> {
    let json = serde_json::to_string(value).map_err(|e| Error::from_reason(e.to_string()))?;
    let global = env.get_global()?;
    let json_obj: Object = global.get_named_property_unchecked("JSON")?;
    let parse: Function<String, RawValue> = json_obj.get_named_property_unchecked("parse")?;
    Ok(unknown(env, parse.call(json)?.0))
}

/// Read a JS value with `JSON.stringify` and serde, as `into_serde` in
/// `automerge-wasm`.
fn from_json_value<T: serde::de::DeserializeOwned>(
    env: &Env,
    value: &Unknown<'_>,
) -> Result<std::result::Result<T, serde_json::Error>> {
    let global = env.get_global()?;
    let json_obj: Object = global.get_named_property_unchecked("JSON")?;
    let stringify: Function<RawValue, String> =
        json_obj.get_named_property_unchecked("stringify")?;
    let json = stringify.call(RawValue(value.raw()))?;
    Ok(serde_json::from_str(&json))
}

fn change_meta_to_js<'env>(env: &'env Env, c: &am::ChangeMetadata<'_>) -> Result<Object<'env>> {
    let mut change = Object::new(env)?;
    change.set_named_property("actor", c.actor.to_string())?;
    match &c.author {
        Some(a) => change.set_named_property("author", a.to_string())?,
        None => change.set_named_property("author", Null)?,
    }
    change.set_named_property("seq", c.seq as f64)?;
    change.set_named_property("startOp", c.start_op as f64)?;
    change.set_named_property("maxOp", c.max_op as f64)?;
    change.set_named_property("time", c.timestamp as f64)?;
    match c.message.as_deref() {
        Some(m) => change.set_named_property("message", m)?,
        None => change.set_named_property("message", Null)?,
    }
    change.set_named_property("deps", export_heads(&c.deps))?;
    change.set_named_property("hash", c.hash.to_string())?;
    if !c.extra.is_empty() {
        change.set_named_property("extraBytes", hex::encode(&c.extra))?;
    }
    Ok(change)
}

fn fragment_to_js<'env>(env: &'env Env, fragment: &am::Fragment) -> Result<Object<'env>> {
    let mut obj = Object::new(env)?;
    obj.set_named_property("head", fragment.head.to_string())?;
    obj.set_named_property("level", fragment.level as f64)?;
    obj.set_named_property("boundary", export_heads(&fragment.boundary))?;
    obj.set_named_property("checkpoints", export_heads(&fragment.checkpoints))?;
    obj.set_named_property("members", export_heads(&fragment.members))?;
    Ok(obj)
}

fn read_hash(
    env: &Env,
    obj: &Object<'_>,
    key: &str,
) -> Result<std::result::Result<ChangeHash, common::heads::BadChangeHash>> {
    let v = interop::get(env, obj, key)?;
    Ok(common::heads::parse_change_hash(
        interop::as_string(&v)?.as_deref(),
    ))
}

fn read_hashes(
    env: &Env,
    obj: &Object<'_>,
    key: &str,
) -> Result<std::result::Result<Vec<ChangeHash>, BadChangeHashes>> {
    let v = interop::get(env, obj, key)?;
    if !interop::is_array(&v)? {
        return Ok(Err(BadChangeHashes::NotArray));
    }
    let arr = as_object(&v)?.unwrap();
    let elems = interop::array_elements(env, &arr)?
        .iter()
        .map(interop::as_string)
        .collect::<Result<Vec<_>>>()?;
    Ok(common::heads::parse_heads(elems))
}

fn fragment_from_js(
    env: &Env,
    obj: &Object<'_>,
) -> Result<std::result::Result<am::Fragment, BadFragmentInput>> {
    let head = match read_hash(env, obj, "head")? {
        Ok(h) => h,
        Err(e) => return Ok(Err(BadFragmentInput::BadHead(e))),
    };
    let level = interop::as_f64(&interop::get(env, obj, "level")?)?;
    let Some(level) = common::fragment::parse_fragment_level(level) else {
        return Ok(Err(BadFragmentInput::BadLevel));
    };
    let boundary = match read_hashes(env, obj, "boundary")? {
        Ok(h) => h,
        Err(e) => return Ok(Err(BadFragmentInput::BadBoundary(e))),
    };
    let checkpoints = match read_hashes(env, obj, "checkpoints")? {
        Ok(h) => h,
        Err(e) => return Ok(Err(BadFragmentInput::BadCheckpoints(e))),
    };
    let members = match read_hashes(env, obj, "members")? {
        Ok(h) => h,
        Err(e) => return Ok(Err(BadFragmentInput::BadMembers(e))),
    };
    Ok(Ok(am::Fragment {
        head,
        level,
        boundary,
        checkpoints,
        members,
    }))
}

impl Automerge {
    fn from_doc(doc: AutoCommit) -> Self {
        Automerge {
            doc,
            freeze: false,
            external_types: HashMap::new(),
        }
    }

    fn derived(&self, doc: AutoCommit) -> Self {
        Automerge {
            doc,
            freeze: self.freeze,
            external_types: self.external_types.clone(),
        }
    }

    fn import(&self, id: &str) -> std::result::Result<(ObjId, ObjType), ImportObj> {
        common::obj::import_obj(&self.doc, id)
    }

    fn exporter<'a, 'env>(&'a self, env: &'env Env) -> Result<Exporter<'a, 'env>> {
        Exporter::new(env, &self.doc, &self.external_types, self.freeze)
    }

    fn import_value<'env>(
        &self,
        env: &'env Env,
        value: &Unknown<'_>,
    ) -> std::result::Result<(Value<'static>, Vec<(Prop, Unknown<'env>)>), InsertObject> {
        match import_scalar(value, None) {
            Ok(val) => Ok((val.into(), vec![])),
            Err(_) => match import_obj(env, value, None) {
                Ok(js_obj) => Ok((js_obj.objtype().into(), js_obj.subvals(env)?)),
                Err(_) => Err(InsertObject::InvalidValue),
            },
        }
    }

    fn subset(
        &mut self,
        env: &Env,
        obj: &ObjId,
        vals: Vec<(Prop, Unknown<'_>)>,
    ) -> std::result::Result<(), InsertObject> {
        for (p, v) in vals {
            let (value, subvals) = self.import_value(env, &v)?;
            let opid = match (p, value) {
                (Prop::Map(s), Value::Object(objtype)) => {
                    Some(self.doc.put_object(obj, s, objtype)?)
                }
                (Prop::Map(s), Value::Scalar(scalar)) => {
                    self.doc.put(obj, s, scalar.into_owned())?;
                    None
                }
                (Prop::Seq(i), Value::Object(objtype)) => {
                    Some(self.doc.insert_object(obj, i, objtype)?)
                }
                (Prop::Seq(i), Value::Scalar(scalar)) => {
                    self.doc.insert(obj, i, scalar.into_owned())?;
                    None
                }
            };
            if let Some(opid) = opid {
                self.subset(env, &opid, subvals)?;
            }
        }
        Ok(())
    }

    fn fill_object(
        &mut self,
        env: &Env,
        opid: &ObjId,
        imported: interop::JsObjType<'_>,
    ) -> std::result::Result<(), InsertObject> {
        if let Some(s) = imported.text() {
            self.doc.splice_text(opid, 0, 0, s)?;
        } else {
            let subvals = imported.subvals(env)?;
            self.subset(env, opid, subvals)?;
        }
        Ok(())
    }

    fn hydrate(
        &self,
        env: &Env,
        value: &Unknown<'_>,
    ) -> std::result::Result<am::hydrate::Value, JsValToHydrate> {
        js_val_to_hydrate(env, &self.external_types, value)
    }

    fn get_value(
        &self,
        obj: &ObjId,
        prop: Prop,
        heads: &Option<Vec<ChangeHash>>,
    ) -> std::result::Result<Option<(Value<'_>, ObjId)>, am::AutomergeError> {
        match heads {
            Some(h) => self.doc.get_at(obj, prop, h),
            None => self.doc.get(obj, prop),
        }
    }

    fn mark_inner(
        &mut self,
        env: &Env,
        obj: String,
        range: Object<'_>,
        name: String,
        value: &Unknown<'_>,
        datatype_name: Option<String>,
    ) -> std::result::Result<(), Mark> {
        let (obj, _) = self.import(&obj)?;
        let start =
            interop::as_f64(&interop::get(env, &range, "start")?)?.ok_or(Mark::InvalidStart)?;
        let end = interop::as_f64(&interop::get(env, &range, "end")?)?.ok_or(Mark::InvalidEnd)?;
        let expand = interop::get(env, &range, "expand")?;
        let expand = if expand.get_type()? == ValueType::Undefined {
            ExpandMark::default()
        } else {
            let s = interop::as_string(&expand)?.ok_or(common::marks::BadExpand)?;
            common::marks::parse_expand(&s)?
        };
        let datatype = datatype(datatype_name)?;
        let value = import_scalar(value, datatype)?;
        self.doc.mark(
            &obj,
            am::marks::Mark::new(name, value, start as usize, end as usize),
            expand,
        )?;
        Ok(())
    }

    fn apply_patches_impl<'env>(
        &mut self,
        env: &'env Env,
        object: Unknown<'env>,
        meta: Unknown<'env>,
    ) -> std::result::Result<(Unknown<'env>, Vec<am::Patch>), ApplyPatch> {
        let mut object = as_object(&object)?.ok_or(ApplyPatch::NotObject)?;
        let shortcut = self.doc.diff_cursor().is_empty();
        let patches = self.doc.diff_incremental();
        let mut cache = Exporter::new(env, &self.doc, &self.external_types, self.freeze)?;
        if shortcut {
            let value = cache.materialize(ROOT, Datatype::Map, None, &meta)?;
            return Ok((value, patches));
        }
        // even if there are no patches we may need to update the meta object
        // which requires that we update the object too
        if meta.get_type()? != ValueType::Undefined {
            object = cache.apply_root_meta(object, &meta)?;
        }
        for p in &patches {
            object = cache.apply_patch(object, p, &meta)?;
        }
        if self.freeze {
            object.freeze()?;
        }
        Ok((object.to_unknown(), patches))
    }
}

#[napi]
impl Automerge {
    /// Kept for compatibility with `automerge-wasm`, where it releases the
    /// document. Here the document is released when it is garbage collected.
    #[napi(catch_unwind)]
    pub fn free(&self) {}

    #[napi(catch_unwind, ts_args_type = "value: any")]
    pub fn init_root_from_hydrate(&mut self, env: &Env, value: Unknown<'_>) -> Result<()> {
        let result = (|| -> std::result::Result<(), InitFromHydrate> {
            let am::hydrate::Value::Map(map) = self.hydrate(env, &value)? else {
                return Err(InitFromHydrate::NotAMap);
            };
            self.doc.init_root_from_hydrate(&map)?;
            Ok(())
        })();
        result.or_throw(env)
    }

    #[napi(catch_unwind)]
    pub fn anonymize(&mut self, env: &Env) -> Result<Automerge> {
        let doc = self
            .doc
            .anonymize()
            .map_err(cerr::Anonymize::from)
            .or_throw(env)?;
        Ok(self.derived(doc))
    }

    #[napi(catch_unwind, js_name = "clone")]
    pub fn clone_doc(&mut self, env: &Env, actor: Option<String>) -> Result<Automerge> {
        let mut automerge = self.derived(self.doc.clone());
        if let Some(s) = actor {
            let actor = common::actor::parse_actor(&s).or_throw(env)?;
            automerge.doc.set_actor(actor);
        }
        Ok(automerge)
    }

    #[napi(catch_unwind)]
    pub fn fork(
        &mut self,
        env: &Env,
        actor: Option<String>,
        heads: Option<Heads>,
    ) -> Result<Automerge> {
        let result = (|| -> std::result::Result<Automerge, cerr::Fork> {
            let heads = heads.map(|h| common::heads::parse_heads(h.into_iter().map(Some)));
            let doc = match heads {
                Some(Ok(heads)) => self.doc.fork_at(&heads)?,
                _ => self.doc.fork(),
            };
            let mut automerge = self.derived(doc);
            if let Some(s) = actor {
                automerge.doc.set_actor(common::actor::parse_actor(&s)?);
            }
            Ok(automerge)
        })();
        result.or_throw(env)
    }

    #[napi(catch_unwind)]
    pub fn pending_ops(&self) -> u32 {
        self.doc.pending_ops() as u32
    }

    #[napi(catch_unwind)]
    pub fn commit(&mut self, message: Option<String>, time: Option<f64>) -> Option<String> {
        let mut commit_opts = CommitOptions::default();
        if let Some(message) = message {
            commit_opts.set_message(message);
        }
        if let Some(time) = time {
            commit_opts.set_time(time as i64);
        }
        self.doc.commit_with(commit_opts).map(|h| h.to_string())
    }

    #[napi(catch_unwind)]
    pub fn merge(&mut self, env: &Env, other: &mut Automerge) -> Result<Heads> {
        let heads = self
            .doc
            .merge(&mut other.doc)
            .map_err(cerr::Merge::from)
            .or_throw(env)?;
        Ok(export_heads(&heads))
    }

    #[napi(catch_unwind)]
    pub fn rollback(&mut self) -> f64 {
        self.doc.rollback() as f64
    }

    #[napi(catch_unwind)]
    pub fn keys(&self, env: &Env, obj: String, heads: Option<Heads>) -> Result<Vec<String>> {
        let result = (|| -> std::result::Result<Vec<String>, Get> {
            let (obj, _) = self.import(&obj)?;
            Ok(match parse_heads(heads)? {
                Some(heads) => self.doc.keys_at(&obj, &heads).collect(),
                None => self.doc.keys(&obj).collect(),
            })
        })();
        result.or_throw(env)
    }

    #[napi(catch_unwind)]
    pub fn text(&self, env: &Env, obj: String, heads: Option<Heads>) -> Result<String> {
        let result = (|| -> std::result::Result<String, Get> {
            let (obj, _) = self.import(&obj)?;
            Ok(match parse_heads(heads)? {
                Some(heads) => self.doc.text_at(&obj, &heads)?,
                None => self.doc.text(&obj)?,
            })
        })();
        result.or_throw(env)
    }

    #[napi(catch_unwind, ts_return_type = "Span[]")]
    pub fn spans<'env>(
        &self,
        env: &'env Env,
        obj: String,
        heads: Option<Heads>,
    ) -> Result<Array<'env>> {
        let result = (|| -> std::result::Result<Array<'env>, GetSpans> {
            let (obj, _) = self.import(&obj)?;
            let spans = match parse_heads(heads)? {
                Some(heads) => self.doc.spans_at(&obj, &heads)?,
                None => self.doc.spans(&obj)?,
            };
            let cache = self.exporter(env).map_err(Export::from)?;
            let mut result = env.create_array(0).map_err(Export::from)?;
            for (i, span) in spans.enumerate() {
                result
                    .set(i as u32, cache.export_span(span)?)
                    .map_err(Export::from)?;
            }
            Ok(result)
        })();
        result.or_throw(env)
    }

    #[napi(
        catch_unwind,
        ts_args_type = "obj: ObjID, start: number, deleteCount: number, text?: string | Array<Value>"
    )]
    pub fn splice(
        &mut self,
        env: &Env,
        obj: String,
        start: f64,
        delete_count: f64,
        text: Option<Either<String, Array<'_>>>,
    ) -> Result<()> {
        let result = (|| -> std::result::Result<(), Splice> {
            let (obj, obj_type) = self.import(&obj)?;
            let start = start as usize;
            let delete_count = delete_count as isize;
            let vals = match text {
                Some(Either::A(t)) => {
                    if obj_type == ObjType::Text {
                        self.doc.splice_text(&obj, start, delete_count, &t)?;
                        return Ok(());
                    }
                    t.chars()
                        .map(|c| ScalarValue::Str(c.to_string().into()))
                        .collect::<Vec<_>>()
                }
                Some(Either::B(array)) => {
                    let mut vals = vec![];
                    for i in 0..array.len() {
                        let raw: RawValue = array.get(i).map_err(ImportValue::from)?.unwrap();
                        vals.push(import_scalar(&unknown(env, raw.0), None)?);
                    }
                    vals
                }
                None => vec![],
            };
            if !vals.is_empty() {
                self.doc.splice(&obj, start, delete_count, vals)?;
            } else {
                // no vals given but we still need to call the text vs splice
                // bc utf16
                match obj_type {
                    ObjType::List => self.doc.splice(&obj, start, delete_count, vals)?,
                    ObjType::Text => self.doc.splice_text(&obj, start, delete_count, "")?,
                    _ => {}
                }
            }
            Ok(())
        })();
        result.or_throw(env)
    }

    #[napi(catch_unwind)]
    pub fn update_text(&mut self, env: &Env, obj: String, new_text: String) -> Result<()> {
        let result = (|| -> std::result::Result<(), cerr::UpdateText> {
            let (obj, obj_type) = self.import(&obj)?;
            if !matches!(obj_type, ObjType::Text) {
                return Err(cerr::UpdateText::ObjectNotText);
            }
            self.doc.update_text(&obj, new_text)?;
            Ok(())
        })();
        result.or_throw(env)
    }

    #[napi(
        catch_unwind,
        ts_args_type = "obj: ObjID, args: Span[], config?: UpdateSpansConfig | undefined | null"
    )]
    pub fn update_spans(
        &mut self,
        env: &Env,
        obj: String,
        args: Vec<Object<'_>>,
        config: Option<Object<'_>>,
    ) -> Result<()> {
        let result = (|| -> std::result::Result<(), UpdateSpans> {
            let (obj, obj_type) = self.import(&obj)?;
            if !matches!(obj_type, ObjType::Text) {
                return Err(UpdateSpans::ObjectNotText);
            }
            let mut spans = Vec::new();
            for (i, v) in args.iter().enumerate() {
                let span = self
                    .import_span(env, v)
                    .map_err(|e| InvalidUpdateSpansArgs::InvalidElement(i, e))?;
                spans.push(span);
            }
            let config = match config {
                Some(c) => import_update_spans_config(env, &c)?,
                None => UpdateSpansConfig::default(),
            };
            self.doc.update_spans(&obj, config, spans)?;
            Ok(())
        })();
        result.or_throw(env)
    }

    #[napi(
        catch_unwind,
        ts_args_type = "obj: ObjID, value: Value, datatype?: Datatype"
    )]
    pub fn push(
        &mut self,
        env: &Env,
        obj: String,
        value: Unknown<'_>,
        datatype_name: Option<String>,
    ) -> Result<()> {
        let result = (|| -> std::result::Result<(), Insert> {
            let (obj, _) = self.import(&obj)?;
            let value = import_scalar(&value, datatype(datatype_name)?)?;
            let index = self.doc.length(&obj);
            self.doc.insert(&obj, index, value)?;
            Ok(())
        })();
        result.or_throw(env)
    }

    #[napi(
        catch_unwind,
        ts_args_type = "obj: ObjID, value: ObjType",
        ts_return_type = "ObjID"
    )]
    pub fn push_object(&mut self, env: &Env, obj: String, value: Unknown<'_>) -> Result<String> {
        let result = (|| -> std::result::Result<String, InsertObject> {
            let (obj, _) = self.import(&obj)?;
            let imported = import_obj(env, &value, None)?;
            let index = self.doc.length(&obj);
            let opid = self.doc.insert_object(&obj, index, imported.objtype())?;
            self.fill_object(env, &opid, imported)?;
            Ok(opid.to_string())
        })();
        result.or_throw(env)
    }

    #[napi(
        catch_unwind,
        ts_args_type = "obj: ObjID, index: number, value: Value, datatype?: Datatype"
    )]
    pub fn insert(
        &mut self,
        env: &Env,
        obj: String,
        index: f64,
        value: Unknown<'_>,
        datatype_name: Option<String>,
    ) -> Result<()> {
        let result = (|| -> std::result::Result<(), Insert> {
            let (obj, _) = self.import(&obj)?;
            let value = import_scalar(&value, datatype(datatype_name)?)?;
            self.doc.insert(&obj, index as usize, value)?;
            Ok(())
        })();
        result.or_throw(env)
    }

    #[napi(
        catch_unwind,
        ts_args_type = "obj: ObjID, index: number, block: {[key: string]: MaterializeValue}"
    )]
    pub fn split_block(
        &mut self,
        env: &Env,
        obj: String,
        index: f64,
        block: Unknown<'_>,
    ) -> Result<()> {
        let result = (|| -> std::result::Result<(), cerr::SplitBlock> {
            let (obj, _) = self.import(&obj)?;
            let block_id = self.doc.split_block(&obj, index as usize)?;
            let hydrate = match self.hydrate(env, &block) {
                Ok(val @ am::hydrate::Value::Map(_)) => val,
                _ => return Err(cerr::SplitBlock::InvalidArgs),
            };
            self.doc.update_object(&block_id, &hydrate)?;
            Ok(())
        })();
        result.or_throw(env)
    }

    #[napi(catch_unwind)]
    pub fn join_block(&mut self, env: &Env, obj: String, index: f64) -> Result<()> {
        let result = (|| -> std::result::Result<(), cerr::Block> {
            let (text, _) = self.import(&obj)?;
            self.doc.join_block(&text, index as usize)?;
            Ok(())
        })();
        result.or_throw(env)
    }

    #[napi(
        catch_unwind,
        ts_args_type = "obj: ObjID, index: number, block: {[key: string]: MaterializeValue}"
    )]
    pub fn update_block(
        &mut self,
        env: &Env,
        obj: String,
        index: f64,
        block: Unknown<'_>,
    ) -> Result<()> {
        let result = (|| -> std::result::Result<(), UpdateBlock> {
            let (text, _) = self.import(&obj)?;
            let new_block = self.doc.replace_block(&text, index as usize)?;
            let new_value = self.hydrate(env, &block)?;
            if !matches!(new_value, am::hydrate::Value::Map(_)) {
                return Err(UpdateBlock::InvalidArgs);
            }
            self.doc.update_object(&new_block, &new_value)?;
            Ok(())
        })();
        result.or_throw(env)
    }

    #[napi(
        catch_unwind,
        ts_return_type = "{ [key: string]: MaterializeValue } | null"
    )]
    pub fn get_block<'env>(
        &mut self,
        env: &'env Env,
        text: String,
        index: f64,
        heads: Option<Heads>,
    ) -> Result<Unknown<'env>> {
        let result = (|| -> std::result::Result<Unknown<'env>, GetBlock> {
            let (text, _) = self.import(&text)?;
            let Some((Value::Object(ObjType::Map), id)) = self.doc.get(&text, index as usize)?
            else {
                return Ok(Null.into_unknown(env).map_err(Export::from)?);
            };
            let heads = parse_heads(heads)?;
            let hydrated = self.doc.hydrate(&id, heads.as_deref())?;
            let cache = self.exporter(env).map_err(Export::from)?;
            Ok(cache.export_hydrate(hydrated)?)
        })();
        result.or_throw(env)
    }

    #[napi(
        catch_unwind,
        ts_args_type = "obj: ObjID, index: number, value: ObjType",
        ts_return_type = "ObjID"
    )]
    pub fn insert_object(
        &mut self,
        env: &Env,
        obj: String,
        index: f64,
        value: Unknown<'_>,
    ) -> Result<String> {
        let result = (|| -> std::result::Result<String, InsertObject> {
            let (obj, _) = self.import(&obj)?;
            let imported = import_obj(env, &value, None)?;
            let opid = self
                .doc
                .insert_object(&obj, index as usize, imported.objtype())?;
            self.fill_object(env, &opid, imported)?;
            Ok(opid.to_string())
        })();
        result.or_throw(env)
    }

    #[napi(
        catch_unwind,
        ts_args_type = "obj: ObjID, prop: Prop, value: Value, datatype?: Datatype"
    )]
    pub fn put(
        &mut self,
        env: &Env,
        obj: String,
        prop: Either<String, f64>,
        value: Unknown<'_>,
        datatype_name: Option<String>,
    ) -> Result<()> {
        let result = (|| -> std::result::Result<(), Insert> {
            let (obj, _) = self.import(&obj)?;
            let value = import_scalar(&value, datatype(datatype_name)?)?;
            self.doc.put(&obj, to_prop(prop), value)?;
            Ok(())
        })();
        result.or_throw(env)
    }

    #[napi(
        catch_unwind,
        ts_args_type = "obj: ObjID, prop: Prop, value: ObjType",
        ts_return_type = "ObjID"
    )]
    pub fn put_object(
        &mut self,
        env: &Env,
        obj: String,
        prop: Either<String, f64>,
        value: Unknown<'_>,
    ) -> Result<String> {
        let result = (|| -> std::result::Result<String, InsertObject> {
            let (obj, _) = self.import(&obj)?;
            let imported = import_obj(env, &value, None)?;
            let opid = self
                .doc
                .put_object(&obj, to_prop(prop), imported.objtype())?;
            self.fill_object(env, &opid, imported)?;
            Ok(opid.to_string())
        })();
        result.or_throw(env)
    }

    #[napi(
        catch_unwind,
        ts_args_type = "obj: ObjID, prop: Prop, value: any",
        ts_return_type = "ObjID"
    )]
    pub fn put_object_from_hydrate(
        &mut self,
        env: &Env,
        obj: String,
        prop: Either<String, f64>,
        value: Unknown<'_>,
    ) -> Result<String> {
        let result = (|| -> std::result::Result<String, BatchCreateObject> {
            let (obj, _) = self.import(&obj)?;
            let hydrate_value = self.hydrate(env, &value)?;
            let opid = self
                .doc
                .batch_create_object(&obj, to_prop(prop), &hydrate_value, false)?;
            Ok(opid.to_string())
        })();
        result.or_throw(env)
    }

    #[napi(
        catch_unwind,
        ts_args_type = "obj: ObjID, index: number, value: any",
        ts_return_type = "ObjID"
    )]
    pub fn insert_object_from_hydrate(
        &mut self,
        env: &Env,
        obj: String,
        index: f64,
        value: Unknown<'_>,
    ) -> Result<String> {
        let result = (|| -> std::result::Result<String, BatchCreateObject> {
            let (obj, _) = self.import(&obj)?;
            let hydrate_value = self.hydrate(env, &value)?;
            let opid = self.doc.batch_create_object(
                &obj,
                Prop::Seq(index as usize),
                &hydrate_value,
                true,
            )?;
            Ok(opid.to_string())
        })();
        result.or_throw(env)
    }

    #[napi(
        catch_unwind,
        ts_args_type = "obj: ObjID, index: number, del: number, values: any"
    )]
    pub fn splice_from_hydrate(
        &mut self,
        env: &Env,
        obj: String,
        index: f64,
        del: f64,
        values: Vec<Unknown<'_>>,
    ) -> Result<()> {
        let result = (|| -> std::result::Result<(), SpliceHydrate> {
            let (obj, _) = self.import(&obj)?;
            let hydrate_values = values
                .iter()
                .map(|v| self.hydrate(env, v))
                .collect::<std::result::Result<Vec<_>, _>>()?;
            self.doc
                .splice(&obj, index as usize, del as i64 as isize, hydrate_values)?;
            Ok(())
        })();
        result.or_throw(env)
    }

    #[napi(catch_unwind, ts_args_type = "obj: ObjID, prop: Prop, value: number")]
    pub fn increment(
        &mut self,
        env: &Env,
        obj: String,
        prop: Either<String, f64>,
        value: f64,
    ) -> Result<()> {
        let result = (|| -> std::result::Result<(), cerr::Increment> {
            let (obj, _) = self.import(&obj)?;
            self.doc.increment(&obj, to_prop(prop), value as i64)?;
            Ok(())
        })();
        result.or_throw(env)
    }

    #[napi(
        catch_unwind,
        ts_args_type = "obj: ObjID, prop: Prop, heads?: Heads",
        ts_return_type = "Value | undefined"
    )]
    pub fn get<'env>(
        &self,
        env: &'env Env,
        obj: String,
        prop: Either<String, f64>,
        heads: Option<Heads>,
    ) -> Result<Unknown<'env>> {
        let result = (|| -> std::result::Result<Unknown<'env>, Get> {
            let (obj, _) = self.import(&obj)?;
            let heads = parse_heads(heads)?;
            match self.get_value(&obj, to_prop(prop), &heads)? {
                Some((value, id)) => match alloc(env, &value)? {
                    (datatype, js_value) if datatype.is_scalar() => Ok(js_value),
                    _ => Ok(env.create_string(id.to_string())?.to_unknown()),
                },
                None => Ok(undefined(env)?),
            }
        })();
        result.or_throw(env)
    }

    #[napi(
        catch_unwind,
        ts_args_type = "obj: ObjID, prop: Prop, heads?: Heads",
        ts_return_type = "FullValue | null"
    )]
    pub fn get_with_type<'env>(
        &self,
        env: &'env Env,
        obj: String,
        prop: Either<String, f64>,
        heads: Option<Heads>,
    ) -> Result<Unknown<'env>> {
        let result = (|| -> std::result::Result<Unknown<'env>, Get> {
            let (obj, _) = self.import(&obj)?;
            let heads = parse_heads(heads)?;
            let mut result = env.create_array(2)?;
            match self.get_value(&obj, to_prop(prop), &heads)? {
                Some((Value::Object(obj_type), obj_id)) => {
                    result.set(0, obj_type.to_string())?;
                    result.set(1, obj_id.to_string())?;
                }
                Some((value @ Value::Scalar(_), _)) => {
                    let (datatype, value) = alloc(env, &value)?;
                    result.set(0, String::from(datatype))?;
                    result.set(1, value)?;
                }
                None => return Ok(Null.into_unknown(env)?),
            }
            Ok(result.to_unknown())
        })();
        result.or_throw(env)
    }

    #[napi(catch_unwind, ts_return_type = "ObjInfo")]
    pub fn obj_info<'env>(
        &self,
        env: &'env Env,
        obj: String,
        heads: Option<Heads>,
    ) -> Result<Object<'env>> {
        let result = (|| -> std::result::Result<Object<'env>, Get> {
            let (obj, _) = self.import(&obj)?;
            let typ = self.doc.object_type(&obj)?;
            let parents = match parse_heads(heads)? {
                Some(heads) => self.doc.parents_at(&obj, &heads),
                None => self.doc.parents(&obj),
            }?;
            let mut result = Object::new(env)?;
            result.set_named_property("id", obj.to_string())?;
            result.set_named_property("type", typ.to_string())?;
            if let Some(path) = parents.visible_path() {
                result.set_named_property("path", interop::export_path(env, &path, None)?)?;
            }
            Ok(result)
        })();
        result.or_throw(env)
    }

    #[napi(
        catch_unwind,
        ts_args_type = "obj: ObjID, arg: Prop, heads?: Heads",
        ts_return_type = "FullValueWithId[]"
    )]
    pub fn get_all<'env>(
        &self,
        env: &'env Env,
        obj: String,
        arg: Either<String, f64>,
        heads: Option<Heads>,
    ) -> Result<Array<'env>> {
        let result = (|| -> std::result::Result<Array<'env>, Get> {
            let (obj, _) = self.import(&obj)?;
            let prop = to_prop(arg);
            let values = match parse_heads(heads)? {
                Some(heads) => self.doc.get_all_at(&obj, prop, &heads),
                None => self.doc.get_all(&obj, prop),
            }?;
            let mut result = env.create_array(0)?;
            for (i, (value, id)) in values.into_iter().enumerate() {
                let mut sub = env.create_array(0)?;
                let (datatype, js_value) = alloc(env, &value)?;
                sub.insert(String::from(datatype))?;
                if value.is_scalar() {
                    sub.insert(js_value)?;
                }
                sub.insert(id.to_string())?;
                result.set(i as u32, sub)?;
            }
            Ok(result)
        })();
        result.or_throw(env)
    }

    #[napi(catch_unwind)]
    pub fn enable_freeze(&mut self, enable: bool) -> bool {
        std::mem::replace(&mut self.freeze, enable)
    }

    #[napi(
        catch_unwind,
        ts_args_type = "datatype: string, construct: Function, deconstruct: (arg: any) => any | undefined"
    )]
    pub fn register_datatype(
        &mut self,
        env: &Env,
        datatype: String,
        construct: Unknown<'_>,
        deconstruct: Unknown<'_>,
    ) -> Result<()> {
        let datatype = Datatype::parse(&datatype)
            .map_err(InvalidDatatype::from)
            .or_throw(env)?;
        if construct.get_type()? != ValueType::Function
            || deconstruct.get_type()? != ValueType::Function
        {
            self.external_types.remove(&datatype);
            return Ok(());
        }
        let construct: Function<RawValue, RawValue> = unsafe { construct.cast() }?;
        let deconstruct: Function<RawValue, RawValue> = unsafe { deconstruct.cast() }?;
        self.external_types.insert(
            datatype,
            Arc::new(ExternalType::new(
                construct.create_ref()?,
                deconstruct.create_ref()?,
            )),
        );
        Ok(())
    }

    #[napi(
        catch_unwind,
        ts_generic_types = "Doc",
        ts_args_type = "obj: Doc, meta?: unknown",
        ts_return_type = "Doc"
    )]
    pub fn apply_patches<'env>(
        &mut self,
        env: &'env Env,
        object: Unknown<'env>,
        meta: Option<Unknown<'env>>,
    ) -> Result<Unknown<'env>> {
        let meta = match meta {
            Some(m) => m,
            None => undefined(env)?,
        };
        let (value, _) = self.apply_patches_impl(env, object, meta).or_throw(env)?;
        Ok(value)
    }

    #[napi(
        catch_unwind,
        ts_generic_types = "Doc",
        ts_args_type = "obj: Doc, meta?: unknown",
        ts_return_type = "{ value: Doc; patches: Patch[] }"
    )]
    pub fn apply_and_return_patches<'env>(
        &mut self,
        env: &'env Env,
        object: Unknown<'env>,
        meta: Option<Unknown<'env>>,
    ) -> Result<Object<'env>> {
        let meta = match meta {
            Some(m) => m,
            None => undefined(env)?,
        };
        let (value, patches) = self.apply_patches_impl(env, object, meta).or_throw(env)?;
        let cache = self.exporter(env)?;
        let patches = cache.export_patches(patches).or_throw(env)?;
        let mut result = Object::new(env)?;
        result.set_named_property("value", value)?;
        result.set_named_property("patches", patches)?;
        Ok(result)
    }

    #[napi(catch_unwind, ts_return_type = "Patch[]")]
    pub fn diff_incremental<'env>(&mut self, env: &'env Env) -> Result<Array<'env>> {
        let patches = self.doc.diff_incremental();
        let cache = self.exporter(env)?;
        cache
            .export_patches(patches)
            .map_err(PopPatches::from)
            .or_throw(env)
    }

    #[napi(catch_unwind)]
    pub fn update_diff_cursor(&mut self) {
        self.doc.update_diff_cursor();
    }

    #[napi(catch_unwind)]
    pub fn reset_diff_cursor(&mut self) {
        self.doc.reset_diff_cursor();
    }

    #[napi(catch_unwind, ts_return_type = "Patch[]")]
    pub fn diff<'env>(
        &mut self,
        env: &'env Env,
        before: Heads,
        after: Heads,
    ) -> Result<Array<'env>> {
        let result = (|| -> std::result::Result<Array<'env>, Diff> {
            let before = common::heads::parse_heads(before.into_iter().map(Some))
                .map_err(Diff::InvalidBeforeHeads)?;
            let after = common::heads::parse_heads(after.into_iter().map(Some))
                .map_err(Diff::InvalidAfterHeads)?;
            let patches = self.doc.diff(&before, &after);
            let cache = self.exporter(env).map_err(Export::from)?;
            Ok(cache.export_patches(patches)?)
        })();
        result.or_throw(env)
    }

    #[napi(
        catch_unwind,
        ts_args_type = "path: Prop[] | string, before: Heads, after: Heads, options?: DiffOptions",
        ts_return_type = "Patch[]"
    )]
    pub fn diff_path<'env>(
        &mut self,
        env: &'env Env,
        path: String,
        before: Heads,
        after: Heads,
        options: Option<DiffOptions>,
    ) -> Result<Array<'env>> {
        let result = (|| -> std::result::Result<Array<'env>, Diff> {
            let obj = self.import(&path)?.0;
            let recursive = options.and_then(|o| o.recursive).unwrap_or(true);
            // automerge-wasm reports both as before heads
            let before = common::heads::parse_heads(before.into_iter().map(Some))
                .map_err(Diff::InvalidBeforeHeads)?;
            let after = common::heads::parse_heads(after.into_iter().map(Some))
                .map_err(Diff::InvalidBeforeHeads)?;
            let patches = self.doc.diff_obj(&obj, &before, &after, recursive)?;
            let cache = self.exporter(env).map_err(Export::from)?;
            Ok(cache.export_patches(patches)?)
        })();
        result.or_throw(env)
    }

    #[napi(catch_unwind)]
    pub fn isolate(&mut self, env: &Env, heads: Heads) -> Result<()> {
        let heads = common::heads::parse_heads(heads.into_iter().map(Some))
            .map_err(cerr::Isolate::from)
            .or_throw(env)?;
        self.doc.isolate(&heads);
        Ok(())
    }

    #[napi(catch_unwind)]
    pub fn integrate(&mut self) {
        self.doc.integrate()
    }

    #[napi(catch_unwind)]
    pub fn length(&self, env: &Env, obj: String, heads: Option<Heads>) -> Result<f64> {
        let result = (|| -> std::result::Result<f64, Get> {
            let (obj, _) = self.import(&obj)?;
            Ok(match parse_heads(heads)? {
                Some(heads) => self.doc.length_at(&obj, &heads) as f64,
                None => self.doc.length(&obj) as f64,
            })
        })();
        result.or_throw(env)
    }

    #[napi(catch_unwind, ts_args_type = "obj: ObjID, prop: Prop")]
    pub fn delete(&mut self, env: &Env, obj: String, prop: Either<String, f64>) -> Result<()> {
        let result = (|| -> std::result::Result<(), Get> {
            let (obj, _) = self.import(&obj)?;
            self.doc.delete(&obj, to_prop(prop))?;
            Ok(())
        })();
        result.or_throw(env)
    }

    #[napi(catch_unwind)]
    pub fn save(&mut self) -> Uint8Array {
        Uint8Array::from(self.doc.save())
    }

    #[napi(catch_unwind)]
    pub fn save_incremental(&mut self) -> Uint8Array {
        Uint8Array::from(self.doc.save_incremental())
    }

    #[napi(catch_unwind)]
    pub fn save_since(&mut self, env: &Env, heads: Heads) -> Result<Uint8Array> {
        let heads = common::heads::parse_heads(heads.into_iter().map(Some)).or_throw(env)?;
        Ok(Uint8Array::from(self.doc.save_after(&heads)))
    }

    #[napi(catch_unwind, js_name = "saveNoCompress")]
    pub fn save_nocompress(&mut self) -> Uint8Array {
        Uint8Array::from(self.doc.save_nocompress())
    }

    #[napi(catch_unwind)]
    pub fn save_and_verify(&mut self, env: &Env) -> Result<Uint8Array> {
        let bytes = self
            .doc
            .save_and_verify()
            .map_err(cerr::Load::from)
            .or_throw(env)?;
        Ok(Uint8Array::from(bytes))
    }

    #[napi(catch_unwind)]
    pub fn load_incremental(&mut self, env: &Env, data: Uint8Array) -> Result<f64> {
        let len = self
            .doc
            .load_incremental(&data)
            .map_err(cerr::Load::from)
            .or_throw(env)?;
        Ok(len as f64)
    }

    #[napi(catch_unwind, ts_args_type = "changes: Change[]")]
    pub fn apply_changes(&mut self, env: &Env, changes: Vec<Uint8Array>) -> Result<()> {
        let result = (|| -> std::result::Result<(), cerr::ApplyChanges> {
            let changes = changes
                .iter()
                .enumerate()
                .map(|(i, c)| {
                    Change::try_from(c.as_ref()).map_err(|e| cerr::BadJSChanges::BadChange(i, e))
                })
                .collect::<std::result::Result<Vec<_>, _>>()?;
            self.doc.apply_changes(changes)?;
            Ok(())
        })();
        result.or_throw(env)
    }

    #[napi(
        catch_unwind,
        ts_args_type = "have_deps: Heads",
        ts_return_type = "Change[]"
    )]
    pub fn get_changes(&mut self, env: &Env, have_deps: Heads) -> Result<Vec<Uint8Array>> {
        let deps = common::heads::parse_heads(have_deps.into_iter().map(Some))
            .map_err(Get::from)
            .or_throw(env)?;
        Ok(uint8arrays(
            self.doc.get_changes(&deps).iter().map(|c| c.raw_bytes()),
        ))
    }

    #[napi(
        catch_unwind,
        ts_args_type = "have_deps: Heads",
        ts_return_type = "ChangeMetadata[]"
    )]
    pub fn get_changes_meta<'env>(
        &mut self,
        env: &'env Env,
        have_deps: Heads,
    ) -> Result<Array<'env>> {
        let deps = common::heads::parse_heads(have_deps.into_iter().map(Some))
            .map_err(Get::from)
            .or_throw(env)?;
        let mut result = env.create_array(0)?;
        for (i, c) in self.doc.get_changes_meta(&deps).iter().enumerate() {
            result.set(i as u32, change_meta_to_js(env, c)?)?;
        }
        Ok(result)
    }

    #[napi(catch_unwind, ts_return_type = "Change | null")]
    pub fn get_change_by_hash(&mut self, env: &Env, hash: String) -> Result<Option<Uint8Array>> {
        let hash = common::heads::parse_change_hash(Some(&hash)).or_throw(env)?;
        Ok(self
            .doc
            .get_change_by_hash(&hash)
            .map(|c| Uint8Array::from(c.raw_bytes().to_vec())))
    }

    #[napi(catch_unwind, ts_return_type = "ChangeMetadata | null")]
    pub fn get_change_meta_by_hash<'env>(
        &mut self,
        env: &'env Env,
        hash: String,
    ) -> Result<Unknown<'env>> {
        let hash = common::heads::parse_change_hash(Some(&hash)).or_throw(env)?;
        match self.doc.get_change_meta_by_hash(&hash) {
            Some(c) => Ok(change_meta_to_js(env, &c)?.to_unknown()),
            None => Null.into_unknown(env),
        }
    }

    #[napi(catch_unwind, ts_return_type = "DecodedChange | null")]
    pub fn get_decoded_change_by_hash<'env>(
        &mut self,
        env: &'env Env,
        hash: String,
    ) -> Result<Unknown<'env>> {
        let hash = common::heads::parse_change_hash(Some(&hash)).or_throw(env)?;
        match self.doc.get_change_by_hash(&hash) {
            Some(c) => {
                let change: am::ExpandedChange = c.decode();
                to_json_value(env, &change)
            }
            None => Null.into_unknown(env),
        }
    }

    #[napi(catch_unwind, ts_return_type = "Change[]")]
    pub fn get_changes_added(&mut self, other: &mut Automerge) -> Vec<Uint8Array> {
        uint8arrays(
            self.doc
                .get_changes_added(&mut other.doc)
                .iter()
                .map(|c| c.raw_bytes()),
        )
    }

    #[napi(
        catch_unwind,
        ts_args_type = "levels?: FragmentLevelRange",
        ts_return_type = "FragmentMeta[]"
    )]
    pub fn get_fragment_metadata<'env>(
        &mut self,
        env: &'env Env,
        levels: Option<Either<f64, Object<'_>>>,
    ) -> Result<Array<'env>> {
        let levels = match levels {
            None => Ok(FragmentLevelRange::all()),
            Some(Either::A(level)) => FragmentLevelRange::level(level),
            Some(Either::B(range)) => {
                let start = interop::as_f64(&interop::get(env, &range, "start")?)?;
                let end = interop::as_f64(&interop::get(env, &range, "end")?)?;
                FragmentLevelRange::range(start, end)
            }
        }
        .map_err(BadFragmentLevelRange::from)
        .or_throw(env)?;
        let mut result = env.create_array(0)?;
        for (i, f) in self.doc.fragments(levels).iter().enumerate() {
            result.set(i as u32, fragment_to_js(env, f)?)?;
        }
        Ok(result)
    }

    #[napi(catch_unwind, ts_return_type = "FragmentMeta | null")]
    pub fn get_fragment_meta<'env>(
        &mut self,
        env: &'env Env,
        head: String,
    ) -> Result<Unknown<'env>> {
        let head = common::heads::parse_change_hash(Some(&head)).or_throw(env)?;
        match self.doc.get_fragment(head) {
            Some(f) => Ok(fragment_to_js(env, &f)?.to_unknown()),
            None => Null.into_unknown(env),
        }
    }

    #[napi(catch_unwind, ts_args_type = "fragments: FragmentMeta[]")]
    pub fn bundle_fragment_metadata(
        &mut self,
        env: &Env,
        fragments: Vec<Object<'_>>,
    ) -> Result<Vec<Uint8Array>> {
        let mut parsed = Vec::new();
        for (i, f) in fragments.iter().enumerate() {
            parsed.push(
                fragment_from_js(env, f)?
                    .map_err(|e| BadFragments(i, e))
                    .or_throw(env)?,
            );
        }
        Ok(uint8arrays(self.doc.bundle_fragments(parsed)))
    }

    #[napi(catch_unwind, ts_args_type = "commits: Commit[]")]
    pub fn add_commits(&mut self, env: &Env, commits: Vec<Object<'_>>) -> Result<()> {
        let mut changes = Vec::new();
        for (i, c) in commits.iter().enumerate() {
            let head = read_hash(env, c, "head")?
                .map_err(|e| AddCommits::BadHead(i, e))
                .or_throw(env)?;
            let parents = read_hashes(env, c, "parents")?
                .map_err(|e| AddCommits::BadParents(i, e))
                .or_throw(env)?;
            let bytes = interop::as_bytes(&interop::get(env, c, "bytes")?)?
                .ok_or_else(|| {
                    Thrown::range(format!(
                        "bad commit input at index {}: bad commit bytes: not a Uint8Array",
                        i
                    ))
                })
                .or_throw(env)?;
            let commit = Commit {
                head,
                parents,
                bytes,
            };
            changes.push(
                commit
                    .into_change()
                    .map_err(AddCommits::from)
                    .or_throw(env)?,
            );
        }
        self.doc
            .apply_changes(changes)
            .map_err(AddCommits::from)
            .or_throw(env)
    }

    #[napi(catch_unwind, ts_args_type = "fragments: Fragment[]")]
    pub fn add_fragments(&mut self, env: &Env, fragments: Vec<Object<'_>>) -> Result<()> {
        let mut bytes = Vec::new();
        for (i, f) in fragments.iter().enumerate() {
            // Parse the metadata so that the whole input shape is validated,
            // even though the bundle bytes are authoritative.
            fragment_from_js(env, f)?
                .map_err(|e| AddFragments::BadInput(i, e))
                .or_throw(env)?;
            let b = interop::as_bytes(&interop::get(env, f, "bytes")?)?
                .ok_or_else(|| {
                    Thrown::range(format!(
                        "bad fragment input at index {}: bad fragment bytes: not a Uint8Array",
                        i
                    ))
                })
                .or_throw(env)?;
            bytes.extend(b);
        }
        self.doc
            .load_incremental(&bytes)
            .map_err(AddFragments::from)
            .or_throw(env)?;
        Ok(())
    }

    #[napi(catch_unwind)]
    pub fn get_heads(&mut self) -> Heads {
        export_heads(&self.doc.get_heads())
    }

    #[napi(catch_unwind)]
    pub fn get_actor_id(&self) -> String {
        self.doc.get_actor().to_string()
    }

    #[napi(catch_unwind, ts_return_type = "Author | null")]
    pub fn get_author(&self) -> Option<String> {
        Some(self.doc.get_author()?.to_string())
    }

    #[napi(catch_unwind, ts_args_type = "author: string, heads: any")]
    pub fn mask_author(&mut self, env: &Env, author: String, heads: Option<Heads>) -> Result<()> {
        let heads = parse_heads(heads).or_throw(env)?.unwrap_or_default();
        let author = common::actor::parse_author(author).or_throw(env)?;
        self.doc.mask_author(author, &heads);
        Ok(())
    }

    #[napi(catch_unwind)]
    pub fn reveal_author(&mut self, env: &Env, author: String) -> Result<()> {
        let author = common::actor::parse_author(author).or_throw(env)?;
        self.doc.reveal_author(&author);
        Ok(())
    }

    #[napi(catch_unwind)]
    pub fn is_author_masked(&self, env: &Env, author: String) -> Result<bool> {
        let author = common::actor::parse_author(author).or_throw(env)?;
        Ok(self.doc.is_author_masked(&author))
    }

    #[napi(catch_unwind)]
    pub fn get_authors(&self) -> Vec<String> {
        self.doc
            .get_authors()
            .iter()
            .map(|a| a.to_string())
            .collect()
    }

    #[napi(catch_unwind, ts_return_type = "Author | null")]
    pub fn get_author_for_actor(&self, env: &Env, actor: String) -> Result<Option<String>> {
        let actor = common::actor::parse_actor(&actor).or_throw(env)?;
        Ok(self.doc.get_author_for_actor(&actor).map(|a| a.to_string()))
    }

    #[napi(catch_unwind)]
    pub fn get_actors_for_author(&self, env: &Env, author: String) -> Result<Vec<String>> {
        let author = common::actor::parse_author(author).or_throw(env)?;
        Ok(self
            .doc
            .get_actors_for_author(&author)
            .iter()
            .map(|a| a.to_string())
            .collect())
    }

    #[napi(catch_unwind)]
    pub fn set_author(&mut self, env: &Env, author: Option<String>) -> Result<()> {
        let author = author
            .map(common::actor::parse_author)
            .transpose()
            .or_throw(env)?;
        self.doc.set_author(author);
        Ok(())
    }

    #[napi(catch_unwind, ts_return_type = "Change | null")]
    pub fn get_last_local_change(&mut self) -> Option<Uint8Array> {
        self.doc
            .get_last_local_change()
            .map(|c| Uint8Array::from(c.raw_bytes().to_vec()))
    }

    #[napi(catch_unwind)]
    pub fn dump(&mut self) {
        self.doc.dump()
    }

    #[napi(catch_unwind)]
    pub fn get_missing_deps(&mut self, env: &Env, heads: Option<Heads>) -> Result<Heads> {
        let heads = parse_heads(heads)
            .map_err(Get::from)
            .or_throw(env)?
            .unwrap_or_default();
        Ok(export_heads(&self.doc.get_missing_deps(&heads)))
    }

    #[napi(catch_unwind, ts_args_type = "state: SyncState, message: SyncMessage")]
    pub fn receive_sync_message(
        &mut self,
        env: &Env,
        state: &mut SyncState,
        message: Uint8Array,
    ) -> Result<()> {
        use automerge::sync::SyncDoc;
        let result = (|| -> std::result::Result<(), cerr::ReceiveSyncMessage> {
            let message = am::sync::Message::decode(&message)?;
            self.doc
                .sync()
                .receive_sync_message(&mut state.0, message)?;
            Ok(())
        })();
        result.or_throw(env)
    }

    #[napi(catch_unwind, ts_return_type = "SyncMessage | null")]
    pub fn generate_sync_message(&mut self, state: &mut SyncState) -> Option<Uint8Array> {
        use automerge::sync::SyncDoc;
        self.doc
            .sync()
            .generate_sync_message(&mut state.0)
            .map(|m| Uint8Array::from(m.encode()))
    }

    #[napi(
        catch_unwind,
        js_name = "toJS",
        ts_args_type = "meta: any",
        ts_return_type = "MaterializeValue"
    )]
    pub fn to_js<'env>(
        &mut self,
        env: &'env Env,
        meta: Option<Unknown<'env>>,
    ) -> Result<Unknown<'env>> {
        let meta = match meta {
            Some(m) => m,
            None => undefined(env)?,
        };
        let mut cache = self.exporter(env)?;
        cache
            .materialize(ROOT, Datatype::Map, None, &meta)
            .or_throw(env)
    }

    #[napi(
        catch_unwind,
        ts_args_type = "obj?: ObjID, heads?: Heads, metadata?: unknown",
        ts_return_type = "MaterializeValue"
    )]
    pub fn materialize<'env>(
        &mut self,
        env: &'env Env,
        obj: Option<String>,
        heads: Option<Heads>,
        meta: Option<Unknown<'env>>,
    ) -> Result<Unknown<'env>> {
        let meta = match meta {
            Some(m) => m,
            None => undefined(env)?,
        };
        let (obj, obj_type) = obj
            .and_then(|o| self.import(&o).ok())
            .unwrap_or((ROOT, ObjType::Map));
        let heads = parse_heads(heads)
            .map_err(Materialize::from)
            .or_throw(env)?;
        self.doc.update_diff_cursor();
        let mut cache = self.exporter(env)?;
        cache
            .materialize(obj, obj_type.into(), heads.as_deref(), &meta)
            .map_err(Materialize::from)
            .or_throw(env)
    }

    #[napi(
        catch_unwind,
        ts_args_type = "obj: ObjID, position: CursorPosition, heads?: Heads, move?: MoveCursor",
        ts_return_type = "Cursor"
    )]
    pub fn get_cursor(
        &mut self,
        env: &Env,
        obj: String,
        position: Either<f64, String>,
        heads: Option<Heads>,
        move_cursor: Option<String>,
    ) -> Result<String> {
        let result = (|| -> std::result::Result<String, cerr::Cursor> {
            let (obj, obj_type) = self.import(&obj).unwrap_or((ROOT, ObjType::Map));
            if obj_type != ObjType::Text {
                return Err(cerr::Cursor::InvalidObjType(obj_type));
            }
            let heads = parse_heads(heads)?;
            let position: CursorPosition = match position {
                Either::A(n) => common::cursor::position_from_number(n),
                Either::B(s) => common::cursor::position_from_str(&s)?,
            };
            let len = match heads {
                Some(ref heads) => self.doc.length_at(&obj, heads),
                None => self.doc.length(&obj),
            };
            let position = common::cursor::clamp_position(position, len);
            let cursor = match move_cursor {
                None => self.doc.get_cursor(obj, position, heads.as_deref())?,
                Some(m) => {
                    let m = common::cursor::parse_move(&m)?;
                    self.doc
                        .get_cursor_moving(obj, position, heads.as_deref(), m)?
                }
            };
            Ok(cursor.to_string())
        })();
        result.or_throw(env)
    }

    #[napi(
        catch_unwind,
        ts_args_type = "obj: ObjID, cursor: Cursor, heads?: Heads"
    )]
    pub fn get_cursor_position(
        &mut self,
        env: &Env,
        obj: String,
        cursor: String,
        heads: Option<Heads>,
    ) -> Result<f64> {
        let result = (|| -> std::result::Result<f64, cerr::Cursor> {
            let (obj, obj_type) = self.import(&obj).unwrap_or((ROOT, ObjType::Map));
            if obj_type != ObjType::Text {
                return Err(cerr::Cursor::InvalidObjType(obj_type));
            }
            let cursor = am::Cursor::try_from(cursor)?;
            let heads = parse_heads(heads)?;
            let position = self
                .doc
                .get_cursor_position(obj, &cursor, heads.as_deref())?;
            Ok(position as f64)
        })();
        result.or_throw(env)
    }

    #[napi(catch_unwind, ts_return_type = "Hash")]
    pub fn empty_change(&mut self, message: Option<String>, time: Option<f64>) -> String {
        let time = time.map(|f| f as i64);
        let options = CommitOptions { message, time };
        hex::encode(self.doc.empty_change(options))
    }

    #[napi(
        catch_unwind,
        ts_args_type = "obj: ObjID, range: MarkRange, name: string, value: Value, datatype?: Datatype"
    )]
    pub fn mark(
        &mut self,
        env: &Env,
        obj: String,
        range: Object<'_>,
        name: String,
        value: Unknown<'_>,
        datatype_name: Option<String>,
    ) -> Result<()> {
        self.mark_inner(env, obj, range, name, &value, datatype_name)
            .or_throw(env)
    }

    #[napi(
        catch_unwind,
        ts_args_type = "obj: ObjID, range: MarkRange, name: string"
    )]
    pub fn unmark(
        &mut self,
        env: &Env,
        obj: String,
        range: Object<'_>,
        name: String,
    ) -> Result<()> {
        let null = Null.into_unknown(env)?;
        self.mark_inner(env, obj, range, name, &null, Some("null".to_string()))
            .or_throw(env)
    }

    #[napi(catch_unwind, ts_return_type = "Mark[]")]
    pub fn marks<'env>(
        &mut self,
        env: &'env Env,
        obj: String,
        heads: Option<Heads>,
    ) -> Result<Array<'env>> {
        let (obj, _) = self
            .import(&obj)
            .map_err(InvalidObjectId::from)
            .or_throw(env)?;
        let heads = parse_heads(heads).or_throw(env)?;
        let marks = match heads {
            Some(heads) => self.doc.marks_at(obj, &heads),
            None => self.doc.marks(obj),
        }
        .map_err(Thrown::error)
        .or_throw(env)?;
        let cache = self.exporter(env)?;
        let mut result = env.create_array(0)?;
        for (i, m) in marks.iter().enumerate() {
            result.set(i as u32, cache.export_mark(m).or_throw(env)?)?;
        }
        Ok(result)
    }

    #[napi(catch_unwind, ts_return_type = "MarkSet")]
    pub fn marks_at<'env>(
        &mut self,
        env: &'env Env,
        obj: String,
        index: f64,
        heads: Option<Heads>,
    ) -> Result<Object<'env>> {
        let (obj, _) = self
            .import(&obj)
            .map_err(InvalidObjectId::from)
            .or_throw(env)?;
        let heads = parse_heads(heads).or_throw(env)?;
        let marks = self
            .doc
            .get_marks(obj, index as usize, heads.as_deref())
            .map_err(Thrown::error)
            .or_throw(env)?;
        let cache = self.exporter(env)?;
        cache.export_marks(&marks).or_throw(env)
    }

    #[napi(catch_unwind)]
    pub fn has_our_changes(&mut self, state: &SyncState) -> bool {
        self.doc.has_our_changes(&state.0)
    }

    #[napi(catch_unwind, ts_return_type = "Hash[]")]
    pub fn topo_history_traversal(&mut self) -> Vec<String> {
        self.doc
            .get_changes(&[])
            .into_iter()
            .map(|c| c.hash().to_string())
            .collect()
    }

    #[napi(catch_unwind, ts_return_type = "Stats")]
    pub fn stats<'env>(&mut self, env: &'env Env) -> Result<Object<'env>> {
        let stats = self.doc.stats();
        let mut result = Object::new(env)?;
        result.set_named_property("numChanges", stats.num_changes as f64)?;
        result.set_named_property("numOps", stats.num_ops as f64)?;
        result.set_named_property("numActors", stats.num_actors as f64)?;
        Ok(result)
    }

    #[napi(catch_unwind, ts_args_type = "hashes: any")]
    pub fn save_bundle(&mut self, env: &Env, hashes: Heads) -> Result<Uint8Array> {
        let result = (|| -> std::result::Result<Uint8Array, cerr::SaveBundle> {
            let hashes = common::heads::parse_heads(hashes.into_iter().map(Some))?;
            let bundle = self
                .doc
                .bundle(hashes.into_iter())
                .map_err(cerr::SaveBundle::DoBundle)?;
            Ok(Uint8Array::from(bundle.bytes().to_vec()))
        })();
        result.or_throw(env)
    }
}

impl Automerge {
    fn import_span(
        &self,
        env: &Env,
        obj: &Object<'_>,
    ) -> std::result::Result<am::Span, InvalidSpan> {
        let type_val = interop::get(env, obj, "type")?;
        let Some(type_str) = interop::as_string(&type_val)? else {
            return Err(InvalidSpan::InvalidType("not a string".to_string()));
        };
        match type_str.as_str() {
            "text" => {
                let text = interop::as_string(&interop::get(env, obj, "value")?)?
                    .ok_or(InvalidSpan::TextNotString)?;
                let marks = interop::get(env, obj, "marks")?;
                let marks = match marks.get_type()? {
                    ValueType::Undefined | ValueType::Null => None,
                    _ => {
                        let marks =
                            as_object(&marks)?.ok_or(InvalidSpan::InvalidMarks(String::new()))?;
                        let mut pairs = Vec::new();
                        for (key, value) in interop::own_entries(env, &marks)? {
                            let value = import_scalar(&value, None)
                                .map_err(|_| InvalidSpan::InvalidMarks(key.clone()))?;
                            pairs.push((key, value));
                        }
                        let marks = am::marks::MarkSet::from_iter(pairs);
                        if marks.is_empty() {
                            None
                        } else {
                            Some(Arc::new(marks))
                        }
                    }
                };
                Ok(am::Span::Text { text, marks })
            }
            "block" => {
                let value = interop::get(env, obj, "value")?;
                let Ok(am::hydrate::Value::Map(map)) = self.hydrate(env, &value) else {
                    return Err(InvalidSpan::BlockNotObject);
                };
                Ok(am::Span::Block(map))
            }
            other => Err(InvalidSpan::InvalidType(other.to_string())),
        }
    }
}

fn import_update_spans_config(
    env: &Env,
    value: &Object<'_>,
) -> std::result::Result<UpdateSpansConfig, UpdateSpans> {
    let parse_expand =
        |v: &Unknown<'_>| -> Result<std::result::Result<ExpandMark, common::marks::BadExpand>> {
            if v.get_type()? == ValueType::Undefined {
                return Ok(Ok(ExpandMark::default()));
            }
            Ok(match interop::as_string(v)? {
                Some(s) => common::marks::parse_expand(&s),
                None => Err(common::marks::BadExpand),
            })
        };
    let default_expand = parse_expand(&interop::get(env, value, "defaultExpand")?)?
        .map_err(ImportUpdateSpansConfig::BadDefaultExpand)?;
    let mut config = UpdateSpansConfig::default().with_default_expand(default_expand);
    let per_mark = interop::get(env, value, "perMarkExpand")?;
    if let Some(obj) = as_object(&per_mark)? {
        for (key, v) in interop::own_entries(env, &obj)? {
            let expand =
                parse_expand(&v)?.map_err(|error| ImportUpdateSpansConfig::BadPerMarkExpand {
                    key: key.clone(),
                    error,
                })?;
            config = config.with_mark_expand(key, expand);
        }
    }
    Ok(config)
}

#[derive(Debug, Default)]
#[napi(object)]
pub struct InitOptions {
    pub actor: Option<String>,
}

#[derive(Debug, Default)]
#[napi(object)]
pub struct LoadOptions {
    pub actor: Option<String>,
    pub unchecked: Option<bool>,
    pub allow_missing_deps: Option<bool>,
    pub convert_immutable_strings_to_text: Option<bool>,
}

#[derive(Debug, Default)]
#[napi(object)]
pub struct DiffOptions {
    pub recursive: Option<bool>,
}

#[napi(catch_unwind, js_name = "create")]
pub fn init(env: &Env, options: Option<InitOptions>) -> Result<Automerge> {
    let options = options.unwrap_or_default();
    let mut doc = AutoCommit::new_with_encoding(TextEncoding::Utf16CodeUnit);
    if let Some(a) = options.actor {
        doc.set_actor(common::actor::parse_actor(&a).or_throw(env)?);
    }
    Ok(Automerge::from_doc(doc))
}

#[napi(catch_unwind)]
pub fn load(env: &Env, data: Uint8Array, options: Option<LoadOptions>) -> Result<Automerge> {
    let options = options.unwrap_or_default();
    let result = (|| -> std::result::Result<AutoCommit, cerr::Load> {
        let verification_mode = if options.unchecked.unwrap_or(false) {
            VerificationMode::DontCheck
        } else {
            VerificationMode::Check
        };
        let on_partial_load = if options.allow_missing_deps.unwrap_or(false) {
            OnPartialLoad::Ignore
        } else {
            OnPartialLoad::Error
        };
        let string_migration = if options.convert_immutable_strings_to_text.unwrap_or(false) {
            StringMigration::ConvertToText
        } else {
            StringMigration::NoMigration
        };
        let mut doc = AutoCommit::load_with_options(
            &data,
            am::LoadOptions::new()
                .on_partial_load(on_partial_load)
                .verification_mode(verification_mode)
                .migrate_strings(string_migration)
                .text_encoding(TextEncoding::Utf16CodeUnit),
        )?;
        if let Some(s) = options.actor {
            doc.set_actor(common::actor::parse_actor(&s)?);
        }
        Ok(doc)
    })();
    Ok(Automerge::from_doc(result.or_throw(env)?))
}

#[derive(Debug)]
#[napi(object)]
pub struct ReleaseInfo {
    pub git_head: String,
    pub cargo_package_name: String,
    pub cargo_package_version: String,
    pub rustc_version: String,
}

/// The equivalent of `wasmReleaseInfo` in `automerge-wasm`.
#[napi(catch_unwind)]
pub fn release_info() -> ReleaseInfo {
    ReleaseInfo {
        git_head: option_env!("GIT_HEAD").unwrap_or("unknown").to_string(),
        cargo_package_name: env!("CARGO_PKG_NAME").to_string(),
        cargo_package_version: env!("CARGO_PKG_VERSION").to_string(),
        rustc_version: env!("CARGO_PKG_RUST_VERSION").to_string(),
    }
}

#[napi(
    catch_unwind,
    ts_args_type = "change: ChangeToEncode",
    ts_return_type = "Change"
)]
pub fn encode_change(env: &Env, change: Unknown<'_>) -> Result<Uint8Array> {
    let change: am::ExpandedChange = from_json_value(env, &change)?
        .map_err(EncodeChange::from)
        .or_throw(env)?;
    let change: Change = change.into();
    Ok(Uint8Array::from(change.raw_bytes().to_vec()))
}

#[napi(
    catch_unwind,
    ts_args_type = "change: Change",
    ts_return_type = "DecodedChange"
)]
pub fn decode_change<'env>(env: &'env Env, change: Uint8Array) -> Result<Unknown<'env>> {
    let change = Change::from_bytes(change.to_vec())
        .map_err(DecodeChange::from)
        .or_throw(env)?;
    let change: am::ExpandedChange = change.decode();
    to_json_value(env, &change)
}

#[napi(catch_unwind, ts_return_type = "DecodedBundle")]
pub fn read_bundle<'env>(env: &'env Env, bundle: Uint8Array) -> Result<Object<'env>> {
    let bundle = automerge::Bundle::try_from(bundle.as_ref())
        .map_err(|e| cerr::ReadBundle(e.to_string()))
        .or_throw(env)?;
    let changes = bundle
        .to_changes()
        .map_err(|e| cerr::ReadBundle(e.to_string()))
        .or_throw(env)?;
    let mut js_changes = env.create_array(0)?;
    for (i, c) in changes.iter().enumerate() {
        let legacy_change = automerge::ExpandedChange::from(c);
        js_changes.set(i as u32, to_json_value(env, &legacy_change)?)?;
    }
    let mut result = Object::new(env)?;
    result.set_named_property("changes", js_changes)?;
    result.set_named_property("deps", export_heads(bundle.deps()))?;
    Ok(result)
}
