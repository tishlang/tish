//! Array builtin methods.
//!
//! This module will contain shared array method implementations.
//! Functions will be migrated here from tish_runtime and tish_eval.

use std::cell::RefCell;
use std::rc::Rc;
use tish_core::Value;

/// Create a new array Value from a Vec of Values.
pub fn from_vec(v: Vec<Value>) -> Value {
    Value::Array(Rc::new(RefCell::new(v)))
}

/// Get the length of an array.
pub fn len(arr: &Value) -> Option<usize> {
    match arr {
        Value::Array(a) => Some(a.borrow().len()),
        _ => None,
    }
}
