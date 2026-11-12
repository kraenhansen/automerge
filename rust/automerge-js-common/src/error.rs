//! Errors of low-level API methods which do not involve any JS-specific
//! failure.
//!
//! Errors which also arise from converting JS values (a property which
//! cannot be read, a value which is not a `Uint8Array`, a data handler which
//! throws, ...) are defined by each binding, wrapping the leaf errors from
//! this crate. Each binding decides which JS error class an error is thrown
//! as; for these that is a `RangeError` in both.

use automerge::{AnonymizeError, AutomergeError, LoadChangeError, ObjType};

use crate::actor::BadActorId;
use crate::cursor::{BadCursorPosition, BadMoveCursor};
use crate::heads::BadChangeHashes;
use crate::obj::{ImportObj, InvalidProp};

#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub struct Anonymize(#[from] AnonymizeError);

#[derive(Debug, thiserror::Error)]
pub enum Fork {
    #[error(transparent)]
    BadActor(#[from] BadActorId),
    #[error(transparent)]
    Automerge(#[from] AutomergeError),
    #[error(transparent)]
    BadChangeHashes(#[from] BadChangeHashes),
}

#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub struct Merge(#[from] AutomergeError);

#[derive(Debug, thiserror::Error)]
pub enum Isolate {
    #[error("bad heads: {0}")]
    Heads(#[from] BadChangeHashes),
    #[error("no heads specified")]
    NoHeads,
}

#[derive(Debug, thiserror::Error)]
pub enum Load {
    #[error(transparent)]
    Automerge(#[from] AutomergeError),
    #[error(transparent)]
    BadActor(#[from] BadActorId),
}

#[derive(Debug, thiserror::Error)]
pub enum Cursor {
    #[error("invalid cursor")]
    InvalidCursor,
    #[error("invalid position - must be an index, 'start' or 'end'")]
    InvalidCursorPosition,
    #[error("invalid move - must be 'before' or 'after'")]
    InvalidMoveCursor,
    #[error("cursors only valid on text - obj type: {0}")]
    InvalidObjType(ObjType),
    #[error("bad heads: {0}")]
    Heads(#[from] BadChangeHashes),
    #[error(transparent)]
    Automerge(#[from] AutomergeError),
}

impl From<BadCursorPosition> for Cursor {
    fn from(_: BadCursorPosition) -> Self {
        Cursor::InvalidCursorPosition
    }
}

impl From<BadMoveCursor> for Cursor {
    fn from(_: BadMoveCursor) -> Self {
        Cursor::InvalidMoveCursor
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ReceiveSyncMessage {
    #[error(transparent)]
    Decode(#[from] automerge::sync::ReadMessageError),
    #[error(transparent)]
    Automerge(#[from] AutomergeError),
}

#[derive(Debug, thiserror::Error)]
pub enum DecodeSyncMessage {
    #[error("could not decode sync message: {0}")]
    ReadMessage(#[from] automerge::sync::ReadMessageError),
}

#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub struct DecodeSyncState(#[from] automerge::sync::DecodeStateError);

#[derive(Debug, thiserror::Error)]
pub enum BadJSChanges {
    #[error("the changes were not an array of Uint8Array")]
    ChangesNotArray,
    #[error("change {0} was not a Uint8Array")]
    ElemNotUint8Array(usize),
    #[error("error loading change {0}: {1}")]
    BadChange(usize, LoadChangeError),
}

#[derive(Debug, thiserror::Error)]
pub enum ApplyChanges {
    #[error(transparent)]
    DecodeChanges(#[from] BadJSChanges),
    #[error("error applying changes: {0}")]
    Apply(#[from] AutomergeError),
}

#[derive(Debug, thiserror::Error)]
pub enum UpdateText {
    #[error("invalid object ID: {0}")]
    ImportObj(#[from] ImportObj),
    #[error(transparent)]
    Automerge(#[from] AutomergeError),
    #[error("object was not a text object")]
    ObjectNotText,
    #[error("update_text is only availalbe for the string representation of text objects")]
    TextRepNotString,
    #[error("value passed to update_text was not a string")]
    ValueNotString,
}

#[derive(Debug, thiserror::Error)]
pub enum Increment {
    #[error("invalid object id: {0}")]
    ImportObj(#[from] ImportObj),
    #[error(transparent)]
    InvalidProp(#[from] InvalidProp),
    #[error("value was not numeric")]
    ValueNotNumeric,
    #[error(transparent)]
    Automerge(#[from] AutomergeError),
}

#[derive(Debug, thiserror::Error)]
pub enum Block {
    #[error("invalid object id: {0}")]
    ImportObj(#[from] ImportObj),
    #[error("block name must be a string")]
    InvalidName,
    #[error("block parents must be an array of strings")]
    InvalidParents,
    #[error("invalid cursor")]
    InvalidCursor,
    #[error(transparent)]
    Automerge(#[from] AutomergeError),
}

#[derive(Debug, thiserror::Error)]
pub enum SplitBlock {
    #[error("invalid object id: {0}")]
    ImportObj(#[from] ImportObj),
    #[error("the block value must be a map")]
    InvalidArgs,
    #[error(transparent)]
    Automerge(#[from] AutomergeError),
    #[error(transparent)]
    UpdateObject(#[from] automerge::error::UpdateObjectError),
}

#[derive(Debug, thiserror::Error)]
pub enum SaveBundle {
    #[error(transparent)]
    BadChangeHashes(#[from] BadChangeHashes),
    #[error("error creating bundle: {0}")]
    DoBundle(AutomergeError),
}

/// An error reading a bundle. Both bindings throw the inner message as it
/// is, without a prefix.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct ReadBundle(pub String);
