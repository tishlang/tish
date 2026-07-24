//! Object builtin methods.
//!
//! This module will contain shared object method implementations.
//! Functions will be migrated here from tishlang_runtime and tishlang_eval.

#[cfg(feature = "portable")]
#[allow(unused_imports)]
use alloc::{borrow::ToOwned, boxed::Box, format, string::{String, ToString}, vec, vec::Vec};

use tishlang_core::Arc;
use tishlang_core::{ObjectData, PropMap, Value, VmRef};

/// Create a new empty object Value.
pub fn new() -> Value {
    Value::empty_object()
}

/// Create a new object Value with a given capacity.
pub fn with_capacity(capacity: usize) -> Value {
    Value::Object(VmRef::new(ObjectData {
        strings: PropMap::with_capacity(capacity),
        symbols: None,
        frozen: false,
    }))
}

/// Get the keys of an object (string keys only; matches `Object.keys` in JS).
pub fn keys(obj: &Value) -> Option<Vec<Arc<str>>> {
    match obj {
        Value::Object(map) => Some(map.borrow().strings.keys().cloned().collect()),
        _ => None,
    }
}

/// Get the values of an object (string-keyed properties only).
pub fn values(obj: &Value) -> Option<Vec<Value>> {
    match obj {
        Value::Object(map) => Some(map.borrow().strings.values().cloned().collect()),
        _ => None,
    }
}
