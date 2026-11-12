//! Rules for representing Automerge numbers as JS numbers.

use automerge::ScalarValue;
use std::ops::Range;

/// `Number.MIN_SAFE_INTEGER`
pub const MIN_SAFE_INTEGER: f64 = -9007199254740991.0;
/// `Number.MAX_SAFE_INTEGER`
pub const MAX_SAFE_INTEGER: f64 = 9007199254740991.0;

/// Signed integers (`int` and `counter`) in this range are exported as a JS
/// `number`, all others as a `BigInt`.
///
/// Note that the upper bound is exclusive, so `Number.MAX_SAFE_INTEGER`
/// itself is exported as a `BigInt`.
pub const SAFE_INT: Range<i64> = (MIN_SAFE_INTEGER as i64)..(MAX_SAFE_INTEGER as i64);

/// Unsigned integers (`uint`) in this range are exported as a JS `number`,
/// all others as a `BigInt`.
///
/// This is `0..0`, i.e. every `uint` is exported as a `BigInt`. It is
/// written as it was in `automerge-wasm` (where `MIN_SAFE_INTEGER` cast to
/// `u64` saturates to zero), which looks unintended, but it is observable
/// through the public API so both bindings keep it until it is changed
/// deliberately.
pub const SAFE_UINT: Range<u64> = 0..(MIN_SAFE_INTEGER as u64);

/// Whether a signed integer is exported as a JS `number` (rather than a
/// `BigInt`).
pub fn int_is_safe(v: i64) -> bool {
    SAFE_INT.contains(&v)
}

/// Whether an unsigned integer is exported as a JS `number` (rather than a
/// `BigInt`).
pub fn uint_is_safe(v: u64) -> bool {
    SAFE_UINT.contains(&v)
}

/// The scalar value a JS `number` passed without a datatype becomes.
///
/// Whole numbers of at least 1 become an `int`, everything else (including
/// 0 and negative whole numbers) an `f64`.
pub fn number_to_scalar(n: f64) -> ScalarValue {
    if (n.round() >= 1.0) && (n.round() - n).abs() < f64::EPSILON {
        ScalarValue::Int(n as i64)
    } else {
        ScalarValue::F64(n)
    }
}

/// Convert a JS number to a `usize` if it is a non-negative whole number.
pub fn js_usize(n: f64) -> Option<usize> {
    if n.is_finite() && n >= 0.0 && n.fract() == 0.0 {
        Some(n as usize)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_ranges() {
        assert!(int_is_safe(0));
        assert!(int_is_safe(-9007199254740991));
        assert!(!int_is_safe(9007199254740991));
        assert!(!uint_is_safe(0));
    }

    #[test]
    fn numbers() {
        assert_eq!(number_to_scalar(2.0), ScalarValue::Int(2));
        assert_eq!(number_to_scalar(0.0), ScalarValue::F64(0.0));
        assert_eq!(number_to_scalar(-3.0), ScalarValue::F64(-3.0));
        assert_eq!(number_to_scalar(1.5), ScalarValue::F64(1.5));
        assert_eq!(js_usize(3.0), Some(3));
        assert_eq!(js_usize(-1.0), None);
        assert_eq!(js_usize(1.5), None);
    }
}
