//! Tish Core - Shared types and utilities for the Tish language.
//!
//! This crate provides the unified Value type and utilities used by both
//! the interpreter (tish_eval) and compiled runtime (tish_runtime).

mod value;
mod json;
mod uri;

pub use value::*;
pub use json::{json_parse, json_stringify};
pub use uri::{percent_decode, percent_encode};
