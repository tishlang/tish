//! Runtime values for the Tish interpreter.

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use tish_ast::Statement;

#[derive(Debug, Clone)]
pub enum Value {
    Number(f64),
    String(Arc<str>),
    Bool(bool),
    Null,
    Array(Rc<Vec<Value>>),
    Object(Rc<HashMap<Arc<str>, Value>>),
    Function {
        params: Vec<Arc<str>>,
        body: Box<Statement>,
    },
    NativePrint,
    NativeParseInt,
    NativeParseFloat,
    NativeIsFinite,
    NativeIsNaN,
    NativeMathAbs,
    NativeMathSqrt,
    NativeMathMin,
    NativeMathMax,
}

impl Value {
    pub fn to_string(&self) -> String {
        match self {
            Value::Number(n) => n.to_string(),
            Value::String(s) => s.to_string(),
            Value::Bool(b) => b.to_string(),
            Value::Null => "null".to_string(),
            Value::Array(arr) => {
                let inner: Vec<String> = arr.iter().map(|v| v.to_string()).collect();
                format!("[{}]", inner.join(", "))
            }
            Value::Object(obj) => {
                let inner: Vec<String> = obj
                    .iter()
                    .map(|(k, v)| format!("{}: {}", k.as_ref(), v.to_string()))
                    .collect();
                format!("{{{}}}", inner.join(", "))
            }
            Value::Function { .. } => "[Function]".to_string(),
            Value::NativePrint => "[NativeFunction: print]".to_string(),
            Value::NativeParseInt => "[NativeFunction: parseInt]".to_string(),
            Value::NativeParseFloat => "[NativeFunction: parseFloat]".to_string(),
            Value::NativeIsFinite => "[NativeFunction: isFinite]".to_string(),
            Value::NativeIsNaN => "[NativeFunction: isNaN]".to_string(),
            Value::NativeMathAbs => "[NativeFunction: Math.abs]".to_string(),
            Value::NativeMathSqrt => "[NativeFunction: Math.sqrt]".to_string(),
            Value::NativeMathMin => "[NativeFunction: Math.min]".to_string(),
            Value::NativeMathMax => "[NativeFunction: Math.max]".to_string(),
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
