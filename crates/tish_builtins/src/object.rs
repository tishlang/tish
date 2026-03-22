//! Object builtin methods.
//!
//! This module will contain shared object method implementations.
//! Functions will be migrated here from tishlang_runtime and tishlang_eval.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use tishlang_core::Value;

/// Create a new empty object Value.
pub fn new() -> Value {
    Value::Object(Rc::new(RefCell::new(HashMap::new())))
}

/// Create a new object Value with a given capacity.
pub fn with_capacity(capacity: usize) -> Value {
    Value::Object(Rc::new(RefCell::new(HashMap::with_capacity(capacity))))
}

/// Get the keys of an object.
pub fn keys(obj: &Value) -> Option<Vec<Arc<str>>> {
    match obj {
        Value::Object(map) => Some(map.borrow().keys().cloned().collect()),
        _ => None,
    }
}

/// Get the values of an object.
pub fn values(obj: &Value) -> Option<Vec<Value>> {
    match obj {
        Value::Object(map) => Some(map.borrow().values().cloned().collect()),
        _ => None,
    }
}
