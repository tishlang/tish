//! Runtime values for the Tish interpreter.

use std::cell::RefCell;
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
    Array(Rc<RefCell<Vec<Value>>>),
    Object(Rc<RefCell<HashMap<Arc<str>, Value>>>),
    Function {
        params: Vec<Arc<str>>,
        rest_param: Option<Arc<str>>,
        body: Box<Statement>,
    },
    NativeConsoleDebug,
    NativeConsoleInfo,
    NativeConsoleLog,
    NativeConsoleWarn,
    NativeConsoleError,
    NativeParseInt,
    NativeParseFloat,
    NativeIsFinite,
    NativeIsNaN,
    NativeMathAbs,
    NativeMathSqrt,
    NativeMathMin,
    NativeMathMax,
    NativeMathFloor,
    NativeMathCeil,
    NativeMathRound,
    NativeJsonParse,
    NativeJsonStringify,
    NativeDecodeURI,
    NativeEncodeURI,
    NativeObjectKeys,
    NativeObjectValues,
    NativeObjectEntries,
    #[cfg(feature = "http")]
    NativeFetch,
    #[cfg(feature = "http")]
    NativeFetchAll,
}

impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Number(n) => {
                if n.is_nan() {
                    write!(f, "NaN")
                } else if *n == f64::INFINITY {
                    write!(f, "Infinity")
                } else if *n == f64::NEG_INFINITY {
                    write!(f, "-Infinity")
                } else {
                    write!(f, "{}", n)
                }
            }
            Value::String(s) => write!(f, "{}", s),
            Value::Bool(b) => write!(f, "{}", b),
            Value::Null => write!(f, "null"),
            Value::Array(arr) => {
                let inner: Vec<String> = arr.borrow().iter().map(|v| v.to_string()).collect();
                write!(f, "[{}]", inner.join(", "))
            }
            Value::Object(obj) => {
                let inner: Vec<String> = obj
                    .borrow()
                    .iter()
                    .map(|(k, v)| format!("{}: {}", k.as_ref(), v))
                    .collect();
                write!(f, "{{{}}}", inner.join(", "))
            }
            Value::Function { .. } => write!(f, "[Function]"),
            Value::NativeConsoleDebug => write!(f, "[NativeFunction: console.debug]"),
            Value::NativeConsoleInfo => write!(f, "[NativeFunction: console.info]"),
            Value::NativeConsoleLog => write!(f, "[NativeFunction: console.log]"),
            Value::NativeConsoleWarn => write!(f, "[NativeFunction: console.warn]"),
            Value::NativeConsoleError => write!(f, "[NativeFunction: console.error]"),
            Value::NativeParseInt => write!(f, "[NativeFunction: parseInt]"),
            Value::NativeParseFloat => write!(f, "[NativeFunction: parseFloat]"),
            Value::NativeIsFinite => write!(f, "[NativeFunction: isFinite]"),
            Value::NativeIsNaN => write!(f, "[NativeFunction: isNaN]"),
            Value::NativeMathAbs => write!(f, "[NativeFunction: Math.abs]"),
            Value::NativeMathSqrt => write!(f, "[NativeFunction: Math.sqrt]"),
            Value::NativeMathMin => write!(f, "[NativeFunction: Math.min]"),
            Value::NativeMathMax => write!(f, "[NativeFunction: Math.max]"),
            Value::NativeMathFloor => write!(f, "[NativeFunction: Math.floor]"),
            Value::NativeMathCeil => write!(f, "[NativeFunction: Math.ceil]"),
            Value::NativeMathRound => write!(f, "[NativeFunction: Math.round]"),
            Value::NativeJsonParse => write!(f, "[NativeFunction: JSON.parse]"),
            Value::NativeJsonStringify => write!(f, "[NativeFunction: JSON.stringify]"),
            Value::NativeDecodeURI => write!(f, "[NativeFunction: decodeURI]"),
            Value::NativeEncodeURI => write!(f, "[NativeFunction: encodeURI]"),
            Value::NativeObjectKeys => write!(f, "[NativeFunction: Object.keys]"),
            Value::NativeObjectValues => write!(f, "[NativeFunction: Object.values]"),
            Value::NativeObjectEntries => write!(f, "[NativeFunction: Object.entries]"),
            #[cfg(feature = "http")]
            Value::NativeFetch => write!(f, "[NativeFunction: fetch]"),
            #[cfg(feature = "http")]
            Value::NativeFetchAll => write!(f, "[NativeFunction: fetchAll]"),
        }
    }
}

impl Value {
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
