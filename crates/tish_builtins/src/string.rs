//! String builtin methods.
//!
//! This module will contain shared string method implementations.
//! Functions will be migrated here from tish_runtime and tish_eval.

use std::sync::Arc;
use tish_core::Value;

/// Create a new string Value from a string slice.
pub fn from_str(s: &str) -> Value {
    Value::String(Arc::from(s))
}

/// Get the length of a string (character count).
pub fn len(s: &Value) -> Option<usize> {
    match s {
        Value::String(str) => Some(str.chars().count()),
        _ => None,
    }
}
