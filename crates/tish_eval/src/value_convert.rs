//! Conversion between tishlang_eval::Value and tishlang_core::Value for opaque method calls.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use tishlang_core::Value as CoreValue;

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
        #[cfg(feature = "http")]
        CoreValue::Promise(p) => Value::CorePromise(Arc::clone(&p)),
        #[cfg(not(feature = "http"))]
        CoreValue::Promise(_) => Value::Null,
        #[cfg(any(feature = "http", feature = "ws"))]
        CoreValue::Function(f) => Value::CoreFn(Rc::clone(&f)),
        #[cfg(not(any(feature = "http", feature = "ws")))]
        CoreValue::Function(_) => Value::Null,
        // tishlang_core gets regex from http or regex features; handle RegExp when it exists
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
