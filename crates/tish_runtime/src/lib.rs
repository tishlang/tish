//! Minimal runtime for Tish compiled output.
//!
//! Provides Value representation, print, and heap/collection support
//! for native-compiled Tish programs.

use std::collections::HashMap;
use std::fmt;

/// Error type for Tish throw/catch.
#[derive(Debug, Clone)]
pub enum TishError {
    Throw(Value),
}

impl fmt::Display for TishError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TishError::Throw(v) => write!(f, "{}", v.to_display_string()),
        }
    }
}

impl std::error::Error for TishError {}
use std::rc::Rc;
use std::sync::Arc;

/// Native function type for first-class functions in compiled code.
pub type NativeFn = Rc<dyn Fn(&[Value]) -> Value>;

/// Runtime value used by compiled Tish programs.
#[derive(Clone)]
pub enum Value {
    Number(f64),
    String(Arc<str>),
    Bool(bool),
    Null,
    Array(Rc<Vec<Value>>),
    Object(Rc<HashMap<Arc<str>, Value>>),
    Function(NativeFn),
}

impl std::fmt::Debug for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Number(n) => write!(f, "Number({})", n),
            Value::String(s) => write!(f, "String({:?})", s.as_ref()),
            Value::Bool(b) => write!(f, "Bool({})", b),
            Value::Null => write!(f, "Null"),
            Value::Array(arr) => write!(f, "Array({:?})", arr.as_ref()),
            Value::Object(obj) => write!(f, "Object({:?})", obj.as_ref()),
            Value::Function(_) => write!(f, "Function"),
        }
    }
}

impl Value {
    pub fn to_display_string(&self) -> String {
        match self {
            Value::Number(n) => n.to_string(),
            Value::String(s) => s.to_string(),
            Value::Bool(b) => b.to_string(),
            Value::Null => "null".to_string(),
            Value::Array(arr) => {
                let inner: Vec<String> = arr.iter().map(|v| v.to_display_string()).collect();
                format!("[{}]", inner.join(", "))
            }
            Value::Object(obj) => {
                let inner: Vec<String> = obj
                    .iter()
                    .map(|(k, v)| format!("{}: {}", k.as_ref(), v.to_display_string()))
                    .collect();
                format!("{{{}}}", inner.join(", "))
            }
            Value::Function(_) => "[Function]".to_string(),
        }
    }

    pub fn is_truthy(&self) -> bool {
        match self {
            Value::Null => false,
            Value::Bool(b) => *b,
            Value::Number(n) => *n != 0.0 && !n.is_nan(),
            Value::String(s) => !s.is_empty(),
            _ => true,
        }
    }

    pub fn strict_eq(&self, other: &Value) -> bool {
        match (self, other) {
            (Value::Number(a), Value::Number(b)) => a == b,
            (Value::String(a), Value::String(b)) => a == b,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Null, Value::Null) => true,
            (Value::Array(a), Value::Array(b)) => Rc::ptr_eq(a, b),
            (Value::Object(a), Value::Object(b)) => Rc::ptr_eq(a, b),
            _ => false,
        }
    }
}

/// Builtin print: prints all arguments space-separated, then newline.
pub fn print(args: &[Value]) {
    let parts: Vec<String> = args.iter().map(Value::to_display_string).collect();
    println!("{}", parts.join(" "));
}

/// Builtin parseInt: parse string to integer. Optional radix 2-36.
pub fn parse_int(args: &[Value]) -> Value {
    let s = args
        .get(0)
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
    if radix >= 2 && radix <= 36 {
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

/// Builtin parseFloat: parse string to float.
pub fn parse_float(args: &[Value]) -> Value {
    let s = args
        .get(0)
        .map(Value::to_display_string)
        .unwrap_or_default();
    let n: f64 = s.trim().parse().unwrap_or(f64::NAN);
    Value::Number(n)
}

/// Builtin isFinite: true if value is finite number.
pub fn is_finite(args: &[Value]) -> Value {
    let b = args.get(0).map_or(false, |v| match v {
        Value::Number(n) => n.is_finite(),
        _ => false,
    });
    Value::Bool(b)
}

/// Builtin isNaN: true if value is NaN or not a number.
pub fn is_nan(args: &[Value]) -> Value {
    let b = args.get(0).map_or(true, |v| match v {
        Value::Number(n) => n.is_nan(),
        _ => true,
    });
    Value::Bool(b)
}

/// Get property from object by string key.
pub fn get_prop(obj: &Value, key: impl AsRef<str>) -> Value {
    let key = key.as_ref();
    match obj {
        Value::Object(map) => {
            let k: Arc<str> = key.into();
            map.get(&k)
                .cloned()
                .unwrap_or(Value::Null)
        }
        Value::Array(arr) => {
            if let Ok(idx) = key.parse::<usize>() {
                arr.get(idx).cloned().unwrap_or(Value::Null)
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
            let key: Arc<str> = match index {
                Value::Number(n) => n.to_string().into(),
                Value::String(s) => Arc::clone(s),
                _ => return Value::Null,
            };
            map.get(&key).cloned().unwrap_or(Value::Null)
        }
        _ => Value::Null,
    }
}
