//! Fragment level ranges.

use std::ops::{Bound, RangeBounds};

use crate::numbers::js_usize;

#[derive(Debug, thiserror::Error)]
pub enum BadFragmentLevelRange {
    #[error("fragment level must be a non-negative integer")]
    InvalidNumber,
    #[error("fragment level range start must be a non-negative integer")]
    InvalidStart,
    #[error("fragment level range end must be a non-negative integer")]
    InvalidEnd,
}

/// The levels passed to `getFragmentMetadata`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FragmentLevelRange {
    pub start: usize,
    pub end: Option<usize>,
}

impl FragmentLevelRange {
    /// All levels (`null` or `undefined`).
    pub fn all() -> Self {
        Self {
            start: 0,
            end: None,
        }
    }

    /// A single level (a number).
    pub fn level(level: f64) -> Result<Self, BadFragmentLevelRange> {
        let level = js_usize(level).ok_or(BadFragmentLevelRange::InvalidNumber)?;
        Ok(Self {
            start: level,
            end: Some(level + 1),
        })
    }

    /// A range of levels (`{ start?: number, end?: number }`).
    pub fn range(start: Option<f64>, end: Option<f64>) -> Result<Self, BadFragmentLevelRange> {
        let start = match start {
            Some(start) => js_usize(start).ok_or(BadFragmentLevelRange::InvalidStart)?,
            None => 0,
        };
        let end = match end {
            Some(end) => Some(js_usize(end).ok_or(BadFragmentLevelRange::InvalidEnd)?),
            None => None,
        };
        Ok(Self { start, end })
    }
}

impl RangeBounds<usize> for FragmentLevelRange {
    fn start_bound(&self) -> Bound<&usize> {
        Bound::Included(&self.start)
    }

    fn end_bound(&self) -> Bound<&usize> {
        match &self.end {
            Some(end) => Bound::Excluded(end),
            None => Bound::Unbounded,
        }
    }
}

/// Parse the `level` of a fragment.
pub fn parse_fragment_level(level: Option<f64>) -> Option<usize> {
    level.and_then(js_usize)
}
