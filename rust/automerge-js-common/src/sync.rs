//! Names used for sync message versions, flags and capabilities.

use automerge::sync::{Capability, MessageFlags, MessageVersion};

#[derive(Debug, thiserror::Error)]
pub enum BadCapabilities {
    #[error("capabilities was not an array")]
    NotArray,
    #[error("element {0} was not a string")]
    ElemNotString(usize),
    #[error("element {0} was not a valid capability: {1}")]
    ElemNotValid(usize, String),
}

/// Parse the `supportedCapabilities` of a decoded sync message. `None`
/// elements stand for JS values which were not strings.
pub fn parse_message_flags<I, S>(flags: I) -> Result<MessageFlags, BadCapabilities>
where
    I: IntoIterator<Item = Option<S>>,
    S: AsRef<str>,
{
    let mut result = MessageFlags::new();
    for (i, v) in flags.into_iter().enumerate() {
        let v = v.ok_or(BadCapabilities::ElemNotString(i))?;
        match v.as_ref() {
            "message-v1" | "message-v2" => {} // accepted but not bitfield flags
            "sync-reset" => result.set(MessageFlags::SYNC_RESET),
            "read-only" => result.set(MessageFlags::READ_ONLY),
            "supports-sync-reset" => result.set(MessageFlags::SUPPORTS_SYNC_RESET),
            other => return Err(BadCapabilities::ElemNotValid(i, other.to_string())),
        }
    }
    Ok(result)
}

/// The names of the flags set in a sync message.
pub fn message_flag_names(flags: MessageFlags) -> Vec<&'static str> {
    let mut result = Vec::new();
    if flags.contains(MessageFlags::SYNC_RESET) {
        result.push("sync-reset");
    }
    if flags.contains(MessageFlags::READ_ONLY) {
        result.push("read-only");
    }
    if flags.contains(MessageFlags::SUPPORTS_SYNC_RESET) {
        result.push("supports-sync-reset");
    }
    result
}

/// Parse the `type` of a decoded sync message. Anything other than `"v2"`
/// is treated as version 1.
pub fn parse_message_version(version: Option<&str>) -> MessageVersion {
    match version {
        Some("v2") => MessageVersion::V2,
        _ => MessageVersion::V1,
    }
}

pub fn message_version_name(version: MessageVersion) -> &'static str {
    match version {
        MessageVersion::V1 => "v1",
        MessageVersion::V2 => "v2",
    }
}

/// Parse a capability in the `theirCapabilities` of an exported sync state.
/// Unknown names are ignored.
///
/// Note that this accepts `"supports-sync-reset"` for
/// [`Capability::SyncReset`] while [`capability_name`] produces
/// `"sync-reset"`; this asymmetry is inherited from `automerge-wasm`.
pub fn parse_capability(name: &str) -> Option<Capability> {
    match name {
        "message-v1" => Some(Capability::MessageV1),
        "message-v2" => Some(Capability::MessageV2),
        "supports-sync-reset" => Some(Capability::SyncReset),
        _ => None,
    }
}

pub fn capability_name(capability: &Capability) -> &'static str {
    match capability {
        Capability::MessageV1 => "message-v1",
        Capability::MessageV2 => "message-v2",
        Capability::SyncReset => "sync-reset",
    }
}
