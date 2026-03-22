//! Tish Core - Shared types and utilities for the Tish language.
//!
//! This crate provides the unified Value type and utilities used by both
//! the interpreter (tishlang_eval) and compiled runtime (tishlang_runtime).

mod console_style;
mod json;
mod macros;
mod uri;
mod value;

pub use console_style::{format_value_styled, format_values_for_console, use_console_colors};
pub use value::*;
pub use json::{json_parse, json_stringify};
pub use uri::{percent_decode, percent_encode};
