//! Shared operations for Tish values.
//! 
//! These functions are used by both the interpreter and compiled runtime,
//! ensuring identical semantics.

use crate::Value;

/// Addition: number + number, or string + string.
/// No implicit coercion - mixed types are an error.
pub fn add(left: &Value, right: &Value) -> Result<Value, String> {
    match (left, right) {
        (Value::Number(a), Value::Number(b)) => Ok(Value::Number(a + b)),
        (Value::String(a), Value::String(b)) => {
            Ok(Value::String(format!("{}{}", a, b).into()))
        }
        // String concatenation with other types (JS-like but explicit)
        (Value::String(a), b) => {
            Ok(Value::String(format!("{}{}", a, b.to_display_string()).into()))
        }
        (a, Value::String(b)) => {
            Ok(Value::String(format!("{}{}", a.to_display_string(), b).into()))
        }
        _ => Err(format!(
            "TypeError: cannot add {} and {}",
            left.type_name(),
            right.type_name()
        )),
    }
}

/// Subtraction: number - number only.
pub fn sub(left: &Value, right: &Value) -> Result<Value, String> {
    match (left, right) {
        (Value::Number(a), Value::Number(b)) => Ok(Value::Number(a - b)),
        _ => Err(format!(
            "TypeError: cannot subtract {} from {}",
            right.type_name(),
            left.type_name()
        )),
    }
}

/// Multiplication: number * number only.
pub fn mul(left: &Value, right: &Value) -> Result<Value, String> {
    match (left, right) {
        (Value::Number(a), Value::Number(b)) => Ok(Value::Number(a * b)),
        _ => Err(format!(
            "TypeError: cannot multiply {} and {}",
            left.type_name(),
            right.type_name()
        )),
    }
}

/// Division: number / number only.
pub fn div(left: &Value, right: &Value) -> Result<Value, String> {
    match (left, right) {
        (Value::Number(a), Value::Number(b)) => Ok(Value::Number(a / b)),
        _ => Err(format!(
            "TypeError: cannot divide {} by {}",
            left.type_name(),
            right.type_name()
        )),
    }
}

/// Modulo: number % number only.
pub fn modulo(left: &Value, right: &Value) -> Result<Value, String> {
    match (left, right) {
        (Value::Number(a), Value::Number(b)) => Ok(Value::Number(a % b)),
        _ => Err(format!(
            "TypeError: cannot compute {} % {}",
            left.type_name(),
            right.type_name()
        )),
    }
}

/// Exponentiation: number ** number only.
pub fn pow(left: &Value, right: &Value) -> Result<Value, String> {
    match (left, right) {
        (Value::Number(a), Value::Number(b)) => Ok(Value::Number(a.powf(*b))),
        _ => Err(format!(
            "TypeError: cannot compute {} ** {}",
            left.type_name(),
            right.type_name()
        )),
    }
}

/// Less than comparison.
pub fn lt(left: &Value, right: &Value) -> Result<Value, String> {
    match (left, right) {
        (Value::Number(a), Value::Number(b)) => Ok(Value::Bool(a < b)),
        (Value::String(a), Value::String(b)) => Ok(Value::Bool(a < b)),
        _ => Err(format!(
            "TypeError: cannot compare {} < {}",
            left.type_name(),
            right.type_name()
        )),
    }
}

/// Less than or equal comparison.
pub fn le(left: &Value, right: &Value) -> Result<Value, String> {
    match (left, right) {
        (Value::Number(a), Value::Number(b)) => Ok(Value::Bool(a <= b)),
        (Value::String(a), Value::String(b)) => Ok(Value::Bool(a <= b)),
        _ => Err(format!(
            "TypeError: cannot compare {} <= {}",
            left.type_name(),
            right.type_name()
        )),
    }
}

/// Greater than comparison.
pub fn gt(left: &Value, right: &Value) -> Result<Value, String> {
    match (left, right) {
        (Value::Number(a), Value::Number(b)) => Ok(Value::Bool(a > b)),
        (Value::String(a), Value::String(b)) => Ok(Value::Bool(a > b)),
        _ => Err(format!(
            "TypeError: cannot compare {} > {}",
            left.type_name(),
            right.type_name()
        )),
    }
}

/// Greater than or equal comparison.
pub fn ge(left: &Value, right: &Value) -> Result<Value, String> {
    match (left, right) {
        (Value::Number(a), Value::Number(b)) => Ok(Value::Bool(a >= b)),
        (Value::String(a), Value::String(b)) => Ok(Value::Bool(a >= b)),
        _ => Err(format!(
            "TypeError: cannot compare {} >= {}",
            left.type_name(),
            right.type_name()
        )),
    }
}

/// Bitwise AND.
pub fn bit_and(left: &Value, right: &Value) -> Result<Value, String> {
    match (left, right) {
        (Value::Number(a), Value::Number(b)) => {
            Ok(Value::Number(((*a as i32) & (*b as i32)) as f64))
        }
        _ => Err(format!(
            "TypeError: cannot compute {} & {}",
            left.type_name(),
            right.type_name()
        )),
    }
}

/// Bitwise OR.
pub fn bit_or(left: &Value, right: &Value) -> Result<Value, String> {
    match (left, right) {
        (Value::Number(a), Value::Number(b)) => {
            Ok(Value::Number(((*a as i32) | (*b as i32)) as f64))
        }
        _ => Err(format!(
            "TypeError: cannot compute {} | {}",
            left.type_name(),
            right.type_name()
        )),
    }
}

/// Bitwise XOR.
pub fn bit_xor(left: &Value, right: &Value) -> Result<Value, String> {
    match (left, right) {
        (Value::Number(a), Value::Number(b)) => {
            Ok(Value::Number(((*a as i32) ^ (*b as i32)) as f64))
        }
        _ => Err(format!(
            "TypeError: cannot compute {} ^ {}",
            left.type_name(),
            right.type_name()
        )),
    }
}

/// Left shift.
pub fn shl(left: &Value, right: &Value) -> Result<Value, String> {
    match (left, right) {
        (Value::Number(a), Value::Number(b)) => {
            Ok(Value::Number(((*a as i32) << (*b as i32)) as f64))
        }
        _ => Err(format!(
            "TypeError: cannot compute {} << {}",
            left.type_name(),
            right.type_name()
        )),
    }
}

/// Right shift (signed).
pub fn shr(left: &Value, right: &Value) -> Result<Value, String> {
    match (left, right) {
        (Value::Number(a), Value::Number(b)) => {
            Ok(Value::Number(((*a as i32) >> (*b as i32)) as f64))
        }
        _ => Err(format!(
            "TypeError: cannot compute {} >> {}",
            left.type_name(),
            right.type_name()
        )),
    }
}

/// Bitwise NOT.
pub fn bit_not(val: &Value) -> Result<Value, String> {
    match val {
        Value::Number(n) => Ok(Value::Number((!(*n as i32)) as f64)),
        _ => Err(format!("TypeError: cannot compute ~{}", val.type_name())),
    }
}

/// Logical NOT.
pub fn logical_not(val: &Value) -> Value {
    Value::Bool(!val.is_truthy())
}

/// Unary negation.
pub fn neg(val: &Value) -> Result<Value, String> {
    match val {
        Value::Number(n) => Ok(Value::Number(-n)),
        _ => Err(format!("TypeError: cannot negate {}", val.type_name())),
    }
}

/// Unary plus.
pub fn pos(val: &Value) -> Result<Value, String> {
    match val {
        Value::Number(n) => Ok(Value::Number(*n)),
        _ => Err(format!("TypeError: cannot apply + to {}", val.type_name())),
    }
}

/// Get property from object/array.
pub fn get_prop(obj: &Value, key: &str) -> Value {
    match obj {
        Value::Object(map) => {
            let k: std::sync::Arc<str> = key.into();
            map.get(&k).cloned().unwrap_or(Value::Null)
        }
        Value::Array(arr) => {
            if key == "length" {
                Value::Number(arr.len() as f64)
            } else if let Ok(idx) = key.parse::<usize>() {
                arr.get(idx).cloned().unwrap_or(Value::Null)
            } else {
                Value::Null
            }
        }
        Value::String(s) => {
            if key == "length" {
                Value::Number(s.chars().count() as f64)
            } else {
                Value::Null
            }
        }
        _ => Value::Null,
    }
}

/// Get index from array or object.
pub fn get_index(obj: &Value, index: &Value) -> Value {
    match obj {
        Value::Array(arr) => {
            let idx = match index {
                Value::Number(n) => *n as usize,
                _ => return Value::Null,
            };
            arr.get(idx).cloned().unwrap_or(Value::Null)
        }
        Value::Object(map) => {
            let key: std::sync::Arc<str> = match index {
                Value::Number(n) => n.to_string().into(),
                Value::String(s) => std::sync::Arc::clone(s),
                _ => return Value::Null,
            };
            map.get(&key).cloned().unwrap_or(Value::Null)
        }
        _ => Value::Null,
    }
}

/// 'in' operator: check if key exists in object/array.
pub fn in_operator(key: &Value, obj: &Value) -> Result<Value, String> {
    let key_str: std::sync::Arc<str> = match key {
        Value::String(s) => std::sync::Arc::clone(s),
        Value::Number(n) => n.to_string().into(),
        _ => return Err(format!("TypeError: 'in' requires string or number key")),
    };
    
    let result = match obj {
        Value::Object(map) => map.contains_key(&key_str),
        Value::Array(arr) => {
            key_str.as_ref() == "length"
                || key_str
                    .parse::<usize>()
                    .ok()
                    .map(|i| i < arr.len())
                    .unwrap_or(false)
        }
        _ => return Err(format!("TypeError: 'in' requires object or array")),
    };
    
    Ok(Value::Bool(result))
}
