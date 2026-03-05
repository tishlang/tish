//! Conversion between tish_eval::Value and tish_core::Value for opaque method calls.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use tish_core::Value as CoreValue;

use crate::value::Value;

/// Convert interpreter Value to core Value. Fails for interpreter-only variants.
pub fn eval_to_core(v: &Value) -> Result<CoreValue, String> {
    match v {
        Value::Number(n) => Ok(CoreValue::Number(*n)),
        Value::String(s) => Ok(CoreValue::String(Arc::clone(s))),
        Value::Bool(b) => Ok(CoreValue::Bool(*b)),
        Value::Null => Ok(CoreValue::Null),
        Value::Array(arr) => {
            let mut out = Vec::new();
            for item in arr.borrow().iter() {
                out.push(eval_to_core(item)?);
            }
            Ok(CoreValue::Array(Rc::new(RefCell::new(out))))
        }
        Value::Object(map) => {
            let mut out = HashMap::new();
            for (k, v) in map.borrow().iter() {
                out.insert(Arc::clone(k), eval_to_core(v)?);
            }
            Ok(CoreValue::Object(Rc::new(RefCell::new(out))))
        }
        Value::Opaque(o) => Ok(CoreValue::Opaque(Arc::clone(o))),
        _ => Err(format!(
            "Cannot pass {:?} to native function (unsupported type)",
            std::mem::discriminant(v)
        )),
    }
}

/// Convert core Value to interpreter Value.
pub fn core_to_eval(v: CoreValue) -> Value {
    match v {
        CoreValue::Number(n) => Value::Number(n),
        CoreValue::String(s) => Value::String(s),
        CoreValue::Bool(b) => Value::Bool(b),
        CoreValue::Null => Value::Null,
        CoreValue::Array(arr) => {
            let mut out = Vec::new();
            for item in arr.borrow().iter() {
                out.push(core_to_eval(item.clone()));
            }
            Value::Array(Rc::new(RefCell::new(out)))
        }
        CoreValue::Object(map) => {
            let mut out = HashMap::new();
            for (k, v) in map.borrow().iter() {
                out.insert(Arc::clone(k), core_to_eval(v.clone()));
            }
            Value::Object(Rc::new(RefCell::new(out)))
        }
        CoreValue::Opaque(o) => Value::Opaque(o),
        CoreValue::Function(_) | CoreValue::Promise(_) => {
            // Not convertible to interpreter Value; caller should not receive these from opaque methods
            Value::Null
        }
        // tish_core gets regex from http or regex features; handle RegExp when it exists
        #[cfg(any(feature = "http", feature = "regex"))]
        CoreValue::RegExp(re) => {
            #[cfg(feature = "regex")]
            {
                Value::RegExp(re)
            }
            #[cfg(not(feature = "regex"))]
            {
                let _ = re;
                Value::Null
            }
        }
    }
}
