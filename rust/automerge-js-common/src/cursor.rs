//! Parsing of cursor positions and cursor movement.

use automerge::{CursorPosition, MoveCursor};

#[derive(Debug, thiserror::Error)]
#[error("cursor position must be an index (number), 'start' or 'end'")]
pub struct BadCursorPosition;

#[derive(Debug, thiserror::Error)]
#[error("move must be 'before' or 'after' - is 'after' by default")]
pub struct BadMoveCursor;

/// A cursor position given as a number. Negative numbers mean the start.
pub fn position_from_number(idx: f64) -> CursorPosition {
    if idx < 0f64 {
        CursorPosition::Start
    } else {
        CursorPosition::Index(idx as usize)
    }
}

/// A cursor position given as a string.
pub fn position_from_str(s: &str) -> Result<CursorPosition, BadCursorPosition> {
    match s {
        "start" => Ok(CursorPosition::Start),
        "end" => Ok(CursorPosition::End),
        _ => Err(BadCursorPosition),
    }
}

/// Positions at or past the end of the sequence refer to its end.
pub fn clamp_position(position: CursorPosition, len: usize) -> CursorPosition {
    match position {
        CursorPosition::Index(i) if i >= len => CursorPosition::End,
        _ => position,
    }
}

pub fn parse_move(s: &str) -> Result<MoveCursor, BadMoveCursor> {
    match s {
        "before" => Ok(MoveCursor::Before),
        "after" => Ok(MoveCursor::After),
        _ => Err(BadMoveCursor),
    }
}
