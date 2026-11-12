//! Parsing of mark options.

use automerge::marks::ExpandMark;

#[derive(Debug, thiserror::Error)]
#[error("expand must be 'left', 'right', 'both', or 'none' - is 'right' by default")]
pub struct BadExpand;

pub fn parse_expand(s: &str) -> Result<ExpandMark, BadExpand> {
    match s {
        "before" => Ok(ExpandMark::Before),
        "after" => Ok(ExpandMark::After),
        "both" => Ok(ExpandMark::Both),
        "none" => Ok(ExpandMark::None),
        _ => Err(BadExpand),
    }
}
