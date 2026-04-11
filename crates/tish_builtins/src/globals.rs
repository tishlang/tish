//! Global builtin functions with signature (args: &[Value]) -> Value.
//!
//! Used by both tishlang_vm (bytecode) and tishlang_runtime (compiled). Keeps tishlang_vm
//! independent of tishlang_runtime.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use tishlang_core::{percent_decode, percent_encode, ObjectMap, Value};

/// Boolean(value) - coerce to bool
pub fn boolean(args: &[Value]) -> Value {
    let v = args.first().unwrap_or(&Value::Null);
    Value::Bool(v.is_truthy())
}

/// decodeURI(str)
pub fn decode_uri(args: &[Value]) -> Value {
    let s = args
        .first()
        .map(Value::to_display_string)
        .unwrap_or_default();
    Value::String(percent_decode(&s).unwrap_or(s).into())
}

/// encodeURI(str)
pub fn encode_uri(args: &[Value]) -> Value {
    let s = args
        .first()
        .map(Value::to_display_string)
        .unwrap_or_default();
    Value::String(percent_encode(&s).into())
}

/// isFinite(value)
pub fn is_finite(args: &[Value]) -> Value {
    Value::Bool(
        args.first()
            .is_some_and(|v| matches!(v, Value::Number(n) if n.is_finite())),
    )
}

/// isNaN(value)
pub fn is_nan(args: &[Value]) -> Value {
    Value::Bool(args.first().is_none_or(|v| {
        matches!(v, Value::Number(n) if n.is_nan()) || !matches!(v, Value::Number(_))
    }))
}

/// Array.isArray(value)
pub fn array_is_array(args: &[Value]) -> Value {
    Value::Bool(matches!(args.first(), Some(Value::Array(_))))
}

/// String(value) — convert value to string (JS String constructor as function).
pub fn string_convert(args: &[Value]) -> Value {
    let v = args.first().unwrap_or(&Value::Null);
    Value::String(v.to_display_string().into())
}

/// String.fromCharCode(...codes)
pub fn string_from_char_code(args: &[Value]) -> Value {
    let s: String = args
        .iter()
        .filter_map(|v| match v {
            Value::Number(n) => char::from_u32(*n as u32),
            _ => None,
        })
        .collect();
    Value::String(s.into())
}

/// Object.keys(obj)
pub fn object_keys(args: &[Value]) -> Value {
    if let Some(Value::Object(obj)) = args.first() {
        let obj_borrow = obj.borrow();
        let keys: Vec<Value> = obj_borrow
            .keys()
            .map(|k| Value::String(Arc::clone(k)))
            .collect();
        Value::Array(Rc::new(RefCell::new(keys)))
    } else {
        Value::Array(Rc::new(RefCell::new(Vec::new())))
    }
}

/// Object.values(obj)
pub fn object_values(args: &[Value]) -> Value {
    if let Some(Value::Object(obj)) = args.first() {
        let obj_borrow = obj.borrow();
        let values: Vec<Value> = obj_borrow.values().cloned().collect();
        Value::Array(Rc::new(RefCell::new(values)))
    } else {
        Value::Array(Rc::new(RefCell::new(Vec::new())))
    }
}

/// Object.entries(obj)
pub fn object_entries(args: &[Value]) -> Value {
    if let Some(Value::Object(obj)) = args.first() {
        let obj_borrow = obj.borrow();
        let entries: Vec<Value> = obj_borrow
            .iter()
            .map(|(k, v)| {
                Value::Array(Rc::new(RefCell::new(vec![
                    Value::String(Arc::clone(k)),
                    v.clone(),
                ])))
            })
            .collect();
        Value::Array(Rc::new(RefCell::new(entries)))
    } else {
        Value::Array(Rc::new(RefCell::new(Vec::new())))
    }
}

/// Object.assign(target, ...sources)
pub fn object_assign(args: &[Value]) -> Value {
    let target = match args.first() {
        Some(Value::Object(obj)) => obj,
        _ => return Value::Null,
    };

    let additional_capacity: usize = args
        .iter()
        .skip(1)
        .map(|source| {
            if let Value::Object(src) = source {
                src.borrow().len()
            } else {
                0
            }
        })
        .sum();

    let mut target_mut = target.borrow_mut();
    target_mut.reserve(additional_capacity);

    for source in args.iter().skip(1) {
        if let Value::Object(src) = source {
            let src_borrow = src.borrow();
            for (k, v) in src_borrow.iter() {
                target_mut.insert(Arc::clone(k), v.clone());
            }
        }
    }
    drop(target_mut);
    Value::Object(Rc::clone(target))
}

/// parseInt(string, radix?)
pub fn parse_int(args: &[Value]) -> Value {
    let s = args
        .first()
        .map(Value::to_display_string)
        .unwrap_or_default();
    let s = s.trim();
    let radix = args
        .get(1)
        .and_then(|v| match v {
            Value::Number(n) => Some(*n as i32),
            _ => None,
        })
        .unwrap_or(10);

    if (2..=36).contains(&radix) {
        let prefix: String = s
            .chars()
            .take_while(|c| *c == '-' || *c == '+' || c.is_digit(radix as u32))
            .collect();
        if let Ok(n) = i64::from_str_radix(&prefix, radix as u32) {
            return Value::Number(n as f64);
        }
    }
    Value::Number(f64::NAN)
}

/// parseFloat(string)
pub fn parse_float(args: &[Value]) -> Value {
    let s = args
        .first()
        .map(Value::to_display_string)
        .unwrap_or_default();
    Value::Number(s.trim().parse().unwrap_or(f64::NAN))
}

/// Object.fromEntries(entries)
pub fn object_from_entries(args: &[Value]) -> Value {
    if let Some(Value::Array(entries)) = args.first() {
        let entries_borrow = entries.borrow();
        let mut obj: ObjectMap = ObjectMap::with_capacity(entries_borrow.len());

        for entry in entries_borrow.iter() {
            if let Value::Array(pair) = entry {
                let pair_borrow = pair.borrow();
                if pair_borrow.len() >= 2 {
                    let key: Arc<str> = match &pair_borrow[0] {
                        Value::String(s) => Arc::clone(s),
                        v => v.to_display_string().into(),
                    };
                    obj.insert(key, pair_borrow[1].clone());
                }
            }
        }

        Value::Object(Rc::new(RefCell::new(obj)))
    } else {
        Value::Object(Rc::new(RefCell::new(ObjectMap::default())))
    }
}
