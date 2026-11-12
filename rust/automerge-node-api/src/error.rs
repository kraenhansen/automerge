//! Errors of the Node-API binding and how they are thrown.
//!
//! napi-rs throws a plain `Error` for any `napi::Error` a function returns.
//! `automerge-wasm` throws a `RangeError` for most errors and a plain string
//! for some, and that is observable through the public API of
//! `@automerge/automerge`. So errors here are converted to a [`Thrown`],
//! which knows which class to throw, and thrown with [`OrThrow::or_throw`]
//! before napi-rs sees them.

use std::fmt::Display;

use automerge::AutomergeError;
use automerge_js_common as common;
use common::heads::{BadChangeHash, BadChangeHashes};
use common::obj::{ImportObj, InvalidProp};
use napi::{Env, Status};

/// How an error is thrown to JS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Class {
    /// `new RangeError(message)`
    Range,
    /// `new Error(message)`
    Error,
    /// The message itself, as a string.
    Str,
}

/// An error on its way to JS.
#[derive(Debug)]
pub(crate) enum Thrown {
    Message(Class, String),
    /// An error from napi-rs, for example an exception thrown by a callback.
    /// It is passed through unchanged.
    Napi(napi::Error),
}

impl Thrown {
    pub(crate) fn range(e: impl Display) -> Self {
        Thrown::Message(Class::Range, e.to_string())
    }

    pub(crate) fn error(e: impl Display) -> Self {
        Thrown::Message(Class::Error, e.to_string())
    }

    pub(crate) fn string(e: impl Display) -> Self {
        Thrown::Message(Class::Str, e.to_string())
    }

    /// Throw this error in `env` and return the error napi-rs expects to be
    /// returned after an exception has been thrown.
    pub(crate) fn throw(self, env: &Env) -> napi::Error {
        let result = match self {
            Thrown::Napi(e) => return e,
            Thrown::Message(Class::Range, msg) => env.throw_range_error(&msg, None),
            Thrown::Message(Class::Error, msg) => env.throw_error(&msg, None),
            Thrown::Message(Class::Str, msg) => env.throw(msg),
        };
        match result {
            Ok(()) => napi::Error::from_status(Status::PendingException),
            Err(e) => e,
        }
    }
}

impl From<napi::Error> for Thrown {
    fn from(e: napi::Error) -> Self {
        Thrown::Napi(e)
    }
}

pub(crate) trait OrThrow<T> {
    /// Throw the error, if any, as the JS class it is thrown as by
    /// `automerge-wasm`.
    fn or_throw(self, env: &Env) -> napi::Result<T>;
}

impl<T, E: Into<Thrown>> OrThrow<T> for Result<T, E> {
    fn or_throw(self, env: &Env) -> napi::Result<T> {
        self.map_err(|e| e.into().throw(env))
    }
}

/// Implements `From<$ty> for Thrown` for errors thrown as `$class`.
macro_rules! thrown_as {
    ($class:ident: $($ty:ty),* $(,)?) => {
        $(
            impl From<$ty> for Thrown {
                fn from(e: $ty) -> Self {
                    Thrown::$class(e)
                }
            }
        )*
    };
}

// The classes below are those `automerge-wasm` throws each error as.
thrown_as!(range:
    common::error::Anonymize,
    common::error::Fork,
    common::error::Merge,
    common::error::Isolate,
    common::error::Load,
    common::error::Cursor,
    common::error::ReceiveSyncMessage,
    common::error::DecodeSyncMessage,
    common::error::ApplyChanges,
    common::error::UpdateText,
    common::error::Increment,
    common::error::SplitBlock,
    common::error::SaveBundle,
    common::error::ReadBundle,
    common::actor::BadActorId,
    common::actor::BadAuthor,
    Get,
    Splice,
    UpdateSpans,
    Insert,
    InsertObject,
    BatchCreateObject,
    InitFromHydrate,
    SpliceHydrate,
    Diff,
    Materialize,
    GetBlock,
    GetSpans,
    Mark,
    UpdateBlock,
    AddCommits,
    AddFragments,
    BadFragmentLevelRange,
    BadFragments,
    DecodeChange,
    EncodeChange,
    ApplyPatch,
    PopPatches,
);
thrown_as!(string:
    common::error::Block,
    common::error::DecodeSyncState,
    BadChangeHash,
    BadChangeHashes,
    InvalidDatatype,
    BadSyncState,
    BadSyncMessage,
    Export,
);

/// An object id that could not be resolved, thrown directly (rather than
/// wrapped in another error) as a string.
#[derive(Debug, thiserror::Error)]
#[error("invalid object ID: {0}")]
pub(crate) struct InvalidObjectId(#[from] pub(crate) ImportObj);

thrown_as!(string: InvalidObjectId);

#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub(crate) struct InvalidDatatype(#[from] pub(crate) common::datatype::InvalidDatatype);

/// Converting a JS value to an Automerge scalar failed.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ImportValue {
    #[error("value not primitive")]
    ValueNotPrimitive,
    #[error("invalid import: {0}")]
    Invalid(String),
    #[error("bignum {0} larger than max {1}")]
    BigIntTooLarge(String, String),
    #[error("bignum {0} smaller than min {1}")]
    BigIntTooSmall(String, String),
    #[error("error calling deconstructor: {0}")]
    CallDataHandler(napi::Error),
    #[error(transparent)]
    Napi(#[from] napi::Error),
}

/// Building JS values failed.
#[derive(Debug, thiserror::Error)]
pub(crate) enum Export {
    #[error("data handler for type {0} did not return a valid object")]
    InvalidDataHandler(String),
    #[error("error calling data handler for type {0}: {1}")]
    CallDataHandler(String, napi::Error),
    #[error(transparent)]
    Automerge(#[from] AutomergeError),
    #[error("invalid root processed")]
    InvalidRoot,
    #[error(transparent)]
    Napi(#[from] napi::Error),
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum Get {
    #[error("invalid object ID: {0}")]
    ImportObj(#[from] ImportObj),
    #[error(transparent)]
    Automerge(#[from] AutomergeError),
    #[error("bad heads: {0}")]
    BadHeads(#[from] BadChangeHashes),
    #[error(transparent)]
    InvalidProp(#[from] InvalidProp),
    #[error(transparent)]
    Export(#[from] Export),
    #[error(transparent)]
    Napi(#[from] napi::Error),
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum Splice {
    #[error("invalid object ID: {0}")]
    ImportObj(#[from] ImportObj),
    #[error(transparent)]
    Automerge(#[from] AutomergeError),
    #[error(transparent)]
    InvalidImport(#[from] ImportValue),
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum UpdateSpans {
    #[error("invalid object ID: {0}")]
    ImportObj(#[from] ImportObj),
    #[error(transparent)]
    Automerge(#[from] AutomergeError),
    #[error("object was not a text object")]
    ObjectNotText,
    #[error(transparent)]
    InvalidArgs(#[from] InvalidUpdateSpansArgs),
    #[error("invalid config: {0}")]
    BadConfig(#[from] ImportUpdateSpansConfig),
    #[error(transparent)]
    Napi(#[from] napi::Error),
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum Insert {
    #[error("invalid object id: {0}")]
    ImportObj(#[from] ImportObj),
    #[error(transparent)]
    Automerge(#[from] AutomergeError),
    #[error(transparent)]
    InvalidImport(#[from] ImportValue),
    #[error(transparent)]
    InvalidDatatype(#[from] InvalidDatatype),
}

impl From<common::datatype::InvalidDatatype> for Insert {
    fn from(e: common::datatype::InvalidDatatype) -> Self {
        Insert::InvalidDatatype(e.into())
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum InsertObject {
    #[error("invalid object id: {0}")]
    ImportObj(#[from] ImportObj),
    #[error("the value to insert must be an object")]
    ValueNotObject,
    #[error(transparent)]
    Automerge(#[from] AutomergeError),
    #[error("given property was not a string or integer")]
    InvalidValue,
    #[error(transparent)]
    Napi(#[from] napi::Error),
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum JsValToHydrate {
    #[error(transparent)]
    ImportValue(#[from] ImportValue),
    #[error("text object had no text")]
    InvalidText,
}

impl From<napi::Error> for JsValToHydrate {
    fn from(e: napi::Error) -> Self {
        JsValToHydrate::ImportValue(e.into())
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum BatchCreateObject {
    #[error("invalid object id: {0}")]
    ImportObj(#[from] ImportObj),
    #[error(transparent)]
    Automerge(#[from] AutomergeError),
    #[error("invalid value")]
    InvalidValue(#[from] JsValToHydrate),
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum InitFromHydrate {
    #[error("value must be a map/object")]
    NotAMap,
    #[error(transparent)]
    Automerge(#[from] AutomergeError),
    #[error(transparent)]
    InvalidValue(#[from] JsValToHydrate),
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum SpliceHydrate {
    #[error("invalid object id: {0}")]
    ImportObj(#[from] ImportObj),
    #[error(transparent)]
    Automerge(#[from] AutomergeError),
    #[error(transparent)]
    InvalidValue(#[from] JsValToHydrate),
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum Diff {
    #[error(transparent)]
    Import(#[from] ImportObj),
    #[error(transparent)]
    Export(#[from] Export),
    #[error(transparent)]
    Automerge(#[from] AutomergeError),
    #[error("invalid before heads: {0}")]
    InvalidBeforeHeads(BadChangeHashes),
    #[error("invalid after heads: {0}")]
    InvalidAfterHeads(BadChangeHashes),
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum Materialize {
    #[error(transparent)]
    Export(#[from] Export),
    #[error("bad heads: {0}")]
    Heads(#[from] BadChangeHashes),
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum GetBlock {
    #[error("invalid object id: {0}")]
    ImportObj(#[from] ImportObj),
    #[error(transparent)]
    Automerge(#[from] AutomergeError),
    #[error(transparent)]
    Export(#[from] Export),
    #[error(transparent)]
    BadHeads(#[from] BadChangeHashes),
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum GetSpans {
    #[error("invalid object id: {0}")]
    ImportObj(#[from] ImportObj),
    #[error(transparent)]
    BadHeads(#[from] BadChangeHashes),
    #[error(transparent)]
    Export(#[from] Export),
    #[error(transparent)]
    Automerge(#[from] AutomergeError),
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum Mark {
    #[error("invalid object id: {0}")]
    ImportObj(#[from] ImportObj),
    #[error(transparent)]
    Automerge(#[from] AutomergeError),
    #[error(transparent)]
    Expand(#[from] common::marks::BadExpand),
    #[error("Invalid mark value: {0}")]
    ImportValue(#[from] ImportValue),
    #[error("start must be a number")]
    InvalidStart,
    #[error("end must be a number")]
    InvalidEnd,
    #[error(transparent)]
    Napi(#[from] napi::Error),
    #[error(transparent)]
    InvalidDatatype(#[from] InvalidDatatype),
}

impl From<common::datatype::InvalidDatatype> for Mark {
    fn from(e: common::datatype::InvalidDatatype) -> Self {
        Mark::InvalidDatatype(e.into())
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum UpdateBlock {
    #[error("invalid object id: {0}")]
    ImportObj(#[from] ImportObj),
    #[error("the updated block args must be a map")]
    InvalidArgs,
    #[error(transparent)]
    Automerge(#[from] AutomergeError),
    #[error(transparent)]
    Update(#[from] automerge::error::UpdateObjectError),
    #[error("invalid value")]
    InvalidValue(#[from] JsValToHydrate),
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum AddCommits {
    #[error("bad commit input at index {0}: bad commit head: {1}")]
    BadHead(usize, BadChangeHash),
    #[error("bad commit input at index {0}: bad commit parents: {1}")]
    BadParents(usize, BadChangeHashes),
    #[error(transparent)]
    BadBytes(#[from] common::commit::BadCommitBytes),
    #[error("error applying commits: {0}")]
    Apply(#[from] AutomergeError),
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum AddFragments {
    #[error("bad fragment input at index {0}: {1}")]
    BadInput(usize, BadFragmentInput),
    #[error("error loading fragment bundles: {0}")]
    Load(#[from] AutomergeError),
}

#[derive(Debug, thiserror::Error)]
#[allow(clippy::enum_variant_names)]
pub(crate) enum BadFragmentInput {
    #[error("bad fragment head: {0}")]
    BadHead(BadChangeHash),
    #[error("bad fragment level")]
    BadLevel,
    #[error("bad fragment boundary: {0}")]
    BadBoundary(BadChangeHashes),
    #[error("bad fragment checkpoints: {0}")]
    BadCheckpoints(BadChangeHashes),
    #[error("bad fragment members: {0}")]
    BadMembers(BadChangeHashes),
}

#[derive(Debug, thiserror::Error)]
#[error("bad fragment at index {0}: {1}")]
pub(crate) struct BadFragments(pub(crate) usize, pub(crate) BadFragmentInput);

#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub(crate) struct BadFragmentLevelRange(#[from] pub(crate) common::fragment::BadFragmentLevelRange);

#[derive(Debug, thiserror::Error)]
pub(crate) enum DecodeChange {
    #[error(transparent)]
    Load(#[from] automerge::LoadChangeError),
    #[error(transparent)]
    Napi(#[from] napi::Error),
}

#[derive(Debug, thiserror::Error)]
#[error("Unable to read JS change: {0}")]
pub(crate) struct EncodeChange(#[from] pub(crate) serde_json::Error);

#[derive(Debug, thiserror::Error)]
pub(crate) enum ApplyPatch {
    #[error(transparent)]
    Export(#[from] Export),
    #[error("cannot delete from a seq")]
    DeleteKeyFromSeq,
    #[error("cannot put key in seq")]
    PutKeyInSeq,
    #[error("cannot increment a non-numeric value")]
    IncrementNonNumeric,
    #[error("cannot increment a key in a seq")]
    IncrementKeyInSeq,
    #[error("cannot increment index in a map")]
    IncrementIndexInMap,
    #[error("cannot insert into a map")]
    InsertInMap,
    #[error("cannot splice into a map")]
    SpliceInMap,
    #[error("cannot splice text into a seq")]
    SpliceTextInSeq,
    #[error("cannot splice text into a map")]
    SpliceTextInMap,
    #[error("cannot put a seq index in a map")]
    PutIdxInMap,
    #[error("cannot mark a span in a map")]
    MarkInMap,
    #[error("patch was not an object")]
    NotObject,
}

impl From<napi::Error> for ApplyPatch {
    fn from(e: napi::Error) -> Self {
        ApplyPatch::Export(e.into())
    }
}

impl From<AutomergeError> for ApplyPatch {
    fn from(e: AutomergeError) -> Self {
        ApplyPatch::Export(e.into())
    }
}

#[derive(Debug, thiserror::Error)]
#[error("unable to build patches: {0}")]
pub(crate) struct PopPatches(#[from] pub(crate) Export);

#[derive(Debug, thiserror::Error)]
pub(crate) enum BadSyncState {
    #[error("bad sharedHeads: {0}")]
    BadSharedHeads(BadChangeHashes),
    #[error("bad lastSentHeads: {0}")]
    BadLastSentHeads(BadChangeHashes),
    #[error("bad theirHeads: {0}")]
    BadTheirHeads(BadChangeHashes),
    #[error("bad theirNeed: {0}")]
    BadTheirNeed(BadChangeHashes),
    #[error("bad theirHave: error loading have at index {0}: {1}")]
    BadTheirHave(usize, BadHave),
    #[error("bad sentHashes: unable to parse hash from {0:?}: {1}")]
    BadSentHashes(String, BadChangeHash),
    #[error("inFlight not a boolean")]
    InFlightNotBoolean,
    #[error("bad theirCapabilities: capabilities was not an array")]
    BadTheirCapabilities,
    #[error("bad theirHave: value was not an array")]
    TheirHaveNotArray,
    #[error(transparent)]
    Napi(#[from] napi::Error),
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum BadHave {
    #[error("bad lastSync: {0}")]
    BadLastSync(BadChangeHashes),
    #[error("bad bloom: unable to decode: {0}")]
    BadBloom(automerge::sync::DecodeBloomError),
    #[error("bad bloom: the value was not a Uint8Array")]
    BloomNotU8Array,
    #[error(transparent)]
    Napi(#[from] napi::Error),
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum BadSyncMessage {
    #[error("unable to read haves: error loading have at index {0}: {1}")]
    BadHaves(usize, BadHave),
    #[error("could not read heads: {0}")]
    BadHeads(BadChangeHashes),
    #[error("could not read need: {0}")]
    BadNeed(BadChangeHashes),
    #[error("bad supported_capabilities: {0}")]
    BadSupportedCapabilities(common::sync::BadCapabilities),
    #[error("unable to read haves: value was not an array")]
    HavesNotArray,
    #[error("no 'changes' property")]
    MissingChanges,
    #[error("could not read changes: chunk list was not an array")]
    ChangesNotArray,
    #[error("could not read changes: element {0} was not a Uint8Array")]
    ChangeNotUint8Array(usize),
    #[error(transparent)]
    Napi(#[from] napi::Error),
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum InvalidSpan {
    #[error("invalid 'type' property: {0}")]
    InvalidType(String),
    #[error("block must be an object")]
    BlockNotObject,
    #[error("'text' property must be a string")]
    TextNotString,
    #[error("invalid marks: value for key {0} could not be converted to a scalar value")]
    InvalidMarks(String),
    #[error(transparent)]
    Napi(#[from] napi::Error),
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum InvalidUpdateSpansArgs {
    #[error("block {0} not a valid block: {1}")]
    InvalidElement(usize, InvalidSpan),
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum ImportUpdateSpansConfig {
    #[error("invalid defaultExpand: {0}")]
    BadDefaultExpand(common::marks::BadExpand),
    #[error("invalid perMarkExpand{key}: {error}")]
    BadPerMarkExpand {
        key: String,
        error: common::marks::BadExpand,
    },
}
