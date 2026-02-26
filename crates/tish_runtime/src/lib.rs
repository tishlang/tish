//! Minimal runtime for Tish compiled output.
//!
//! Provides Value representation, print, and heap/collection support
//! for native-compiled Tish programs.

use std::collections::HashMap;
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
    let idx = match index {
        Value::Number(n) => *n as usize,
        _ => return Value::Null,
    };
    match obj {
        Value::Array(arr) => arr.get(idx).cloned().unwrap_or(Value::Null),
        Value::Object(map) => {
            let key: Arc<str> = idx.to_string().into();
            map.get(&key).cloned().unwrap_or(Value::Null)
        }
        _ => Value::Null,
    }
}
