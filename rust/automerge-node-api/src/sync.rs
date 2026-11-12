//! Sync state and sync messages.

use std::collections::BTreeSet;

use automerge as am;
use automerge::sync::{Have, Message, MessageVersion};
use automerge::ChangeHash;
use automerge_js_common as common;
use common::heads::BadChangeHashes;
use napi::bindgen_prelude::*;
use napi::{Env, JsValue, ValueType};
use napi_derive::napi;

use crate::error::{BadHave, BadSyncMessage, BadSyncState, OrThrow, Thrown};
use crate::interop::{self, export_heads};

#[napi]
pub struct SyncState(pub(crate) am::sync::State);

#[napi]
impl SyncState {
    #[napi(catch_unwind, getter)]
    pub fn shared_heads(&self) -> Vec<String> {
        export_heads(&self.0.shared_heads)
    }

    #[napi(catch_unwind, getter)]
    pub fn last_sent_heads(&self) -> Vec<String> {
        export_heads(&self.0.last_sent_heads)
    }

    #[napi(catch_unwind, setter)]
    pub fn set_last_sent_heads(&mut self, env: &Env, heads: Vec<String>) -> Result<()> {
        self.0.last_sent_heads =
            common::heads::parse_heads(heads.into_iter().map(Some)).or_throw(env)?;
        Ok(())
    }

    /// Set the hashes sent to the peer, as an object of `hash: true`.
    #[napi(catch_unwind, setter, ts_args_type = "value: Heads")]
    pub fn set_sent_hashes(&mut self, env: &Env, hashes: Object<'_>) -> Result<()> {
        let mut set = BTreeSet::new();
        for (key, _) in interop::own_entries(env, &hashes)? {
            let hash = common::heads::parse_change_hash(Some(&key))
                .map_err(Thrown::error)
                .or_throw(env)?;
            set.insert(hash);
        }
        self.0.sent_hashes = set;
        Ok(())
    }

    #[napi(catch_unwind, getter)]
    pub fn read_only(&self) -> bool {
        self.0.read_only
    }

    #[napi(catch_unwind, setter)]
    pub fn set_read_only(&mut self, read_only: bool) {
        self.0.set_read_only(read_only);
    }

    #[napi(catch_unwind, getter)]
    pub fn peer_read_only(&self) -> bool {
        self.0.is_peer_read_only()
    }

    #[napi(catch_unwind, js_name = "clone")]
    pub fn clone_state(&self) -> SyncState {
        SyncState(self.0.clone())
    }

    /// Kept for compatibility with `automerge-wasm`, where it releases the
    /// state. Here the state is released when it is garbage collected.
    #[napi(catch_unwind)]
    pub fn free(&self) {}
}

fn hash_array<'env>(env: &'env Env, hashes: &[ChangeHash]) -> Result<Array<'env>> {
    Array::from_vec(env, export_heads(hashes))
}

fn read_heads(
    value: &Unknown<'_>,
    env: &Env,
) -> std::result::Result<Vec<ChangeHash>, BadChangeHashes> {
    let Some(obj) = (if interop::is_array(value).unwrap_or(false) {
        interop::as_object(value).ok().flatten()
    } else {
        None
    }) else {
        return Err(BadChangeHashes::NotArray);
    };
    let elems = interop::array_elements(env, &obj).map_err(|_| BadChangeHashes::NotArray)?;
    common::heads::parse_heads(elems.iter().map(|v| interop::as_string(v).ok().flatten()))
}

fn export_haves<'env>(env: &'env Env, haves: &[Have]) -> Result<Array<'env>> {
    let mut result = env.create_array(0)?;
    for (i, have) in haves.iter().enumerate() {
        let mut obj = Object::new(env)?;
        obj.set_named_property("lastSync", hash_array(env, &have.last_sync)?)?;
        obj.set_named_property("bloom", Uint8Array::from(have.bloom.to_bytes()))?;
        result.set(i as u32, obj)?;
    }
    Ok(result)
}

fn import_have(env: &Env, value: &Unknown<'_>) -> std::result::Result<Have, BadHave> {
    let obj = interop::as_object(value)?.ok_or(BadHave::BadLastSync(BadChangeHashes::NotArray))?;
    let last_sync =
        read_heads(&interop::get(env, &obj, "lastSync")?, env).map_err(BadHave::BadLastSync)?;
    let bloom =
        interop::as_bytes(&interop::get(env, &obj, "bloom")?)?.ok_or(BadHave::BloomNotU8Array)?;
    let bloom = bloom.as_slice().try_into().map_err(BadHave::BadBloom)?;
    Ok(Have { last_sync, bloom })
}

fn import_haves(
    env: &Env,
    value: &Unknown<'_>,
) -> Result<Option<Vec<std::result::Result<Have, BadHave>>>> {
    if !interop::is_array(value)? {
        return Ok(None);
    }
    let obj = interop::as_object(value)?.unwrap();
    Ok(Some(
        interop::array_elements(env, &obj)?
            .iter()
            .map(|v| import_have(env, v))
            .collect(),
    ))
}

#[napi(catch_unwind, js_name = "initSyncState")]
pub fn init_sync_state() -> SyncState {
    SyncState(am::sync::State::new())
}

#[napi(catch_unwind, ts_return_type = "JsSyncState")]
pub fn export_sync_state<'env>(env: &'env Env, state: &SyncState) -> Result<Object<'env>> {
    let state = &state.0;
    let opt_heads = |h: &Option<Vec<ChangeHash>>| -> Result<Unknown<'env>> {
        match h {
            Some(h) => Ok(hash_array(env, h)?.to_unknown()),
            None => Null.into_unknown(env),
        }
    };
    let mut result = Object::new(env)?;
    result.set_named_property("sharedHeads", hash_array(env, &state.shared_heads)?)?;
    result.set_named_property("lastSentHeads", hash_array(env, &state.last_sent_heads)?)?;
    result.set_named_property("theirHeads", opt_heads(&state.their_heads)?)?;
    result.set_named_property("theirNeed", opt_heads(&state.their_need)?)?;
    let their_have = match &state.their_have {
        Some(have) => export_haves(env, have)?.to_unknown(),
        None => Null.into_unknown(env)?,
    };
    result.set_named_property("theirHave", their_have)?;
    let mut sent = Object::new(env)?;
    for hash in &state.sent_hashes {
        sent.set_named_property(&hash.to_string(), true)?;
    }
    result.set_named_property("sentHashes", sent)?;
    result.set_named_property("inFlight", state.in_flight)?;
    result.set_named_property("haveResponded", state.have_responded)?;
    if let Some(caps) = &state.their_capabilities {
        let names: Vec<&str> = caps.iter().map(common::sync::capability_name).collect();
        result.set_named_property("theirCapabilities", names)?;
    }
    result.set_named_property("readOnly", state.read_only)?;
    result.set_named_property("peerReadOnly", state.peer_read_only)?;
    Ok(result)
}

fn import_sync_state_inner(
    env: &Env,
    value: &Object<'_>,
) -> std::result::Result<am::sync::State, BadSyncState> {
    let get = |k: &str| interop::get(env, value, k);
    let opt_heads =
        |v: Unknown<'_>| -> std::result::Result<Option<Vec<ChangeHash>>, BadChangeHashes> {
            if v.get_type().map(|t| t == ValueType::Null).unwrap_or(false) {
                Ok(None)
            } else {
                read_heads(&v, env).map(Some)
            }
        };
    let shared_heads =
        read_heads(&get("sharedHeads")?, env).map_err(BadSyncState::BadSharedHeads)?;
    let last_sent_heads =
        read_heads(&get("lastSentHeads")?, env).map_err(BadSyncState::BadLastSentHeads)?;
    let their_heads = opt_heads(get("theirHeads")?).map_err(BadSyncState::BadTheirHeads)?;
    let their_need = opt_heads(get("theirNeed")?).map_err(BadSyncState::BadTheirNeed)?;
    let their_have_val = get("theirHave")?;
    let their_have = if their_have_val.get_type()? == ValueType::Null {
        None
    } else {
        let haves = import_haves(env, &their_have_val)?.ok_or(BadSyncState::TheirHaveNotArray)?;
        Some(
            haves
                .into_iter()
                .enumerate()
                .map(|(i, h)| h.map_err(|e| BadSyncState::BadTheirHave(i, e)))
                .collect::<std::result::Result<Vec<_>, _>>()?,
        )
    };
    let mut sent_hashes = BTreeSet::new();
    if let Some(obj) = interop::as_object(&get("sentHashes")?)? {
        for (key, v) in interop::own_entries(env, &obj)? {
            if interop::as_bool(&v)? == Some(true) {
                let hash = common::heads::parse_change_hash(Some(&key))
                    .map_err(|e| BadSyncState::BadSentHashes(key.clone(), e))?;
                sent_hashes.insert(hash);
            }
        }
    }
    let in_flight = interop::as_bool(&get("inFlight")?)?.ok_or(BadSyncState::InFlightNotBoolean)?;
    let have_responded = interop::as_bool(&get("haveResponded")?)?.unwrap_or(false);
    let caps = get("theirCapabilities")?;
    let their_capabilities = if caps.get_type()? == ValueType::Undefined {
        None
    } else {
        if !interop::is_array(&caps)? {
            return Err(BadSyncState::BadTheirCapabilities);
        }
        let obj = interop::as_object(&caps)?.unwrap();
        let mut result = Vec::new();
        for v in interop::array_elements(env, &obj)? {
            if let Some(s) = interop::as_string(&v)? {
                if let Some(c) = common::sync::parse_capability(&s) {
                    result.push(c);
                }
            }
        }
        Some(result)
    };
    let read_only = interop::as_bool(&get("readOnly")?)?.unwrap_or(false);
    let peer_read_only = interop::as_bool(&get("peerReadOnly")?)?.unwrap_or(false);
    Ok(am::sync::State {
        shared_heads,
        last_sent_heads,
        their_heads,
        their_need,
        their_have,
        sent_hashes,
        in_flight,
        have_responded,
        their_capabilities,
        read_only,
        peer_read_only,
        needs_reset: false,
    })
}

#[napi(catch_unwind, ts_args_type = "state: JsSyncState")]
pub fn import_sync_state(env: &Env, state: Object<'_>) -> Result<SyncState> {
    import_sync_state_inner(env, &state)
        .map(SyncState)
        .or_throw(env)
}

#[napi(catch_unwind)]
pub fn encode_sync_state(state: &SyncState) -> Uint8Array {
    Uint8Array::from(state.0.encode())
}

#[napi(catch_unwind)]
pub fn decode_sync_state(env: &Env, data: Uint8Array) -> Result<SyncState> {
    am::sync::State::decode(&data)
        .map(SyncState)
        .map_err(common::error::DecodeSyncState::from)
        .or_throw(env)
}

fn encode_sync_message_inner(
    env: &Env,
    value: &Object<'_>,
) -> std::result::Result<Message, BadSyncMessage> {
    let get = |k: &str| interop::get(env, value, k);
    let heads = read_heads(&get("heads")?, env).map_err(BadSyncMessage::BadHeads)?;
    let need = read_heads(&get("need")?, env).map_err(BadSyncMessage::BadNeed)?;
    let have = import_haves(env, &get("have")?)?
        .ok_or(BadSyncMessage::HavesNotArray)?
        .into_iter()
        .enumerate()
        .map(|(i, h)| h.map_err(|e| BadSyncMessage::BadHaves(i, e)))
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let flags_val = get("supportedCapabilities")?;
    let flags = match flags_val.get_type()? {
        ValueType::Undefined | ValueType::Null => None,
        _ => {
            if !interop::is_array(&flags_val)? {
                return Err(BadSyncMessage::BadSupportedCapabilities(
                    common::sync::BadCapabilities::NotArray,
                ));
            }
            let obj = interop::as_object(&flags_val)?.unwrap();
            let elems = interop::array_elements(env, &obj)?;
            let strings = elems
                .iter()
                .map(interop::as_string)
                .collect::<Result<Vec<_>>>()?;
            Some(
                common::sync::parse_message_flags(strings)
                    .map_err(BadSyncMessage::BadSupportedCapabilities)?,
            )
        }
    };
    let version =
        common::sync::parse_message_version(interop::as_string(&get("type")?)?.as_deref());
    let changes_val = get("changes")?;
    if changes_val.get_type()? == ValueType::Undefined {
        return Err(BadSyncMessage::MissingChanges);
    }
    if !interop::is_array(&changes_val)? {
        return Err(BadSyncMessage::ChangesNotArray);
    }
    let obj = interop::as_object(&changes_val)?.unwrap();
    let mut changes = Vec::new();
    for (i, v) in interop::array_elements(env, &obj)?.iter().enumerate() {
        changes.push(interop::as_bytes(v)?.ok_or(BadSyncMessage::ChangeNotUint8Array(i))?);
    }
    Ok(Message {
        heads,
        need,
        have,
        changes: changes.into(),
        flags,
        version,
    })
}

#[napi(catch_unwind, ts_args_type = "message: DecodedSyncMessage")]
pub fn encode_sync_message(env: &Env, message: Object<'_>) -> Result<Uint8Array> {
    let message = encode_sync_message_inner(env, &message).or_throw(env)?;
    Ok(Uint8Array::from(message.encode()))
}

#[napi(catch_unwind, ts_return_type = "DecodedSyncMessage")]
pub fn decode_sync_message<'env>(env: &'env Env, msg: Uint8Array) -> Result<Object<'env>> {
    let msg = Message::decode(&msg)
        .map_err(common::error::DecodeSyncMessage::from)
        .or_throw(env)?;
    let mut obj = Object::new(env)?;
    obj.set_named_property("heads", hash_array(env, &msg.heads)?)?;
    obj.set_named_property("need", hash_array(env, &msg.need)?)?;
    obj.set_named_property("have", export_haves(env, &msg.have)?)?;
    let mut changes = env.create_array(0)?;
    for (i, c) in msg.changes.iter().enumerate() {
        changes.set(i as u32, Uint8Array::from(c.to_vec()))?;
    }
    obj.set_named_property("changes", changes)?;
    let version = match msg.version {
        MessageVersion::V1 => "v1",
        MessageVersion::V2 => "v2",
    };
    obj.set_named_property("type", version)?;
    if let Some(flags) = msg.flags {
        obj.set_named_property(
            "supportedCapabilities",
            common::sync::message_flag_names(flags),
        )?;
    }
    Ok(obj)
}
