//! Common helper functions used across builtin implementations.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use tishlang_core::{ObjectMap, Value};

/// Normalize an array index, handling negative indices.
/// Returns a valid index within bounds or the default value.
pub fn normalize_index(idx: &Value, len: i64, default: usize) -> usize {
    match idx {
        Value::Number(n) => {
            let n = *n as i64;
            if n < 0 {
                (len + n).max(0) as usize
            } else {
                n.min(len) as usize
            }
        }
        _ => default,
    }
}

/// Create an error object with a single "error" field.
pub fn make_error_value(e: impl std::fmt::Display) -> Value {
    let mut obj = ObjectMap::with_capacity(1);
    obj.insert(Arc::from("error"), Value::String(e.to_string().into()));
    Value::Object(Rc::new(RefCell::new(obj)))
}

/// Extract a number from a Value, returning None for non-numbers.
pub fn extract_num(v: Option<&Value>) -> Option<f64> {
    v.and_then(|val| match val {
        Value::Number(n) => Some(*n),
        _ => None,
    })
}
