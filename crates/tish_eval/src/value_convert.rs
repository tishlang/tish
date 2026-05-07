//! Conversion between tishlang_eval::Value and tishlang_core::Value for opaque method calls.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

#[cfg(feature = "regex")]
use tishlang_core::TishRegExp;
use ahash::AHashMap;
use tishlang_core::{ObjectData, ObjectMap, Value as CoreValue, VmRef};

use crate::value::{EvalObjectData, PropMap, Value};

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
            Ok(CoreValue::Array(VmRef::new(out)))
        }
        Value::Object(map) => {
            let b = map.borrow();
            let mut strings = ObjectMap::default();
            for (k, v) in b.strings.iter() {
                strings.insert(Arc::clone(k), eval_to_core(v)?);
            }
            let symbols = if let Some(ss) = &b.symbols {
                let mut sm = AHashMap::default();
                for (id, v) in ss.iter() {
                    sm.insert(*id, eval_to_core(v)?);
                }
                Some(sm)
            } else {
                None
            };
            Ok(CoreValue::Object(VmRef::new(ObjectData { strings, symbols })))
        }
        Value::Symbol(s) => Ok(CoreValue::Symbol(Arc::clone(s))),
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
            let b = map.borrow();
            let mut out = PropMap::default();
            for (k, v) in b.strings.iter() {
                out.insert(Arc::clone(k), core_to_eval(v.clone()));
            }
            let mut eod = EvalObjectData::from_strings(out);
            if let Some(ss) = &b.symbols {
                let mut es = AHashMap::default();
                for (id, v) in ss.iter() {
                    es.insert(*id, core_to_eval(v.clone()));
                }
                eod.symbols = Some(es);
            }
            Value::Object(Rc::new(RefCell::new(eod)))
        }
        CoreValue::Symbol(s) => Value::Symbol(Arc::clone(&s)),
        CoreValue::Opaque(o) => Value::Opaque(o),
        #[cfg(feature = "http")]
        CoreValue::Promise(p) => Value::CorePromise(Arc::clone(&p)),
        #[cfg(not(feature = "http"))]
        CoreValue::Promise(_) => Value::Null,
        // `CoreNativeFn` is feature-gated (Rc vs Arc), so use Clone::clone
        // which works for either.
        CoreValue::Function(f) => Value::CoreFn(f.clone()),
        // tishlang_core gets regex from http or regex features; handle RegExp when it exists
        #[cfg(any(feature = "http", feature = "regex"))]
        CoreValue::RegExp(re) => {
            #[cfg(feature = "regex")]
            {
                // Core uses `VmRef<TishRegExp>` (potentially `Arc<Mutex>`),
                // interpreter uses `Rc<RefCell<TishRegExp>>`. Clone the
                // inner state across so the two storage shapes can coexist.
                let inner: TishRegExp = re.borrow().clone();
                Value::RegExp(Rc::new(RefCell::new(inner)))
            }
            #[cfg(not(feature = "regex"))]
            {
                let _ = re;
                Value::Null
            }
        }
    }
}
