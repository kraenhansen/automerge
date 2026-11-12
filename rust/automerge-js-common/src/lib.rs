//! Engine-independent logic shared by `automerge-wasm` and
//! `automerge-node-api`.
//!
//! Nothing in this crate knows about JavaScript values. Each binding converts
//! JS values to the plain Rust types these functions take (strings, numbers,
//! slices) with the primitives of its binding generator, and converts the
//! errors defined here to the JS error class the public API exposes.
#![warn(missing_debug_implementations, rust_2018_idioms, unreachable_pub)]

pub mod actor;
pub mod commit;
pub mod cursor;
pub mod datatype;
pub mod error;
pub mod fragment;
pub mod heads;
pub mod marks;
pub mod numbers;
pub mod obj;
pub mod sync;

pub use datatype::Datatype;
