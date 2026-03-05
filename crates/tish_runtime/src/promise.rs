//! Promise static methods for compiled Tish (resolve, reject, all, race).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use tish_core::{Value, TishPromise};

/// Promise.resolve(value) - returns the value (immediate resolve).
pub fn promise_resolve(args: &[Value]) -> Value {
    args.first().cloned().unwrap_or(Value::Null)
}

/// Promise.reject(value) - returns value as "rejected" placeholder.
/// Note: await on this in native compile may not throw; use try/catch in interpreter for full support.
pub fn promise_reject(args: &[Value]) -> Value {
    args.first().cloned().unwrap_or(Value::Null)
}

/// Promise.all(iterable) - awaits each Promise in the array, returns array of resolved values.
pub fn promise_all(args: &[Value]) -> Value {
    match args.first() {
        Some(Value::Array(arr)) => {
            let arr = arr.borrow();
            let resolved: Vec<Value> = arr
                .iter()
                .map(|v| {
                    if let Value::Promise(p) = v {
                        match p.block_until_settled() {
                            Ok(val) => val,
                            Err(rejection) => rejection,
                        }
                    } else {
                        v.clone()
                    }
                })
                .collect();
            Value::Array(Rc::new(RefCell::new(resolved)))
        }
        Some(v) => v.clone(),
        None => Value::Null,
    }
}

/// Promise.race(iterable) - returns first element of array.
pub fn promise_race(args: &[Value]) -> Value {
    match args.first() {
        Some(Value::Array(arr)) => arr
            .borrow()
            .first()
            .cloned()
            .unwrap_or(Value::Null),
        _ => Value::Null,
    }
}

/// Build the Promise object with resolve, reject, all, race static methods.
pub fn promise_object() -> Value {
    let mut map: HashMap<Arc<str>, Value> = HashMap::new();
    map.insert(
        Arc::from("resolve"),
        Value::Function(Rc::new(|args: &[Value]| promise_resolve(args))),
    );
    map.insert(
        Arc::from("reject"),
        Value::Function(Rc::new(|args: &[Value]| promise_reject(args))),
    );
    map.insert(
        Arc::from("all"),
        Value::Function(Rc::new(|args: &[Value]| promise_all(args))),
    );
    map.insert(
        Arc::from("race"),
        Value::Function(Rc::new(|args: &[Value]| promise_race(args))),
    );
    Value::Object(Rc::new(RefCell::new(map)))
}
