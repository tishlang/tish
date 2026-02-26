//! Tish Core - Shared types and operations for the Tish language.
//!
//! This crate provides the unified Value type and operations used by both
//! the interpreter (tish_eval) and compiled runtime (tish_runtime).

mod value;
mod ops;
mod json;
mod uri;

pub use value::*;
pub use ops::*;
pub use json::{json_parse, json_stringify};
pub use uri::{percent_decode, percent_encode, percent_encode_component};
