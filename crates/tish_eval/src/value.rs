//! Runtime values for the Tish interpreter.
//!
//! This module defines the interpreter's `Value` type, which includes variants
//! like `Function`, `Native`, and `Serve` that hold AST or interpreter-specific
//! data. The compiled runtime uses `tish_core::Value` instead, which has a
//! different shape (no AST-carrying variants). The split is intentional.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use tish_ast::{Expr, Statement};
#[cfg(feature = "http")]
use tish_core::{NativeFn as CoreNativeFn, TishPromise};
use tish_core::TishOpaque;

#[cfg(feature = "http")]
pub use crate::promise::PromiseResolver;
#[cfg(feature = "regex")]
pub use crate::regex::TishRegExp;

/// Native function type - takes args, returns Result<Value, String>
pub type NativeFn = fn(&[Value]) -> Result<Value, String>;

#[derive(Clone)]
pub enum Value {
    Number(f64),
    String(Arc<str>),
    Bool(bool),
    Null,
    Array(Rc<RefCell<Vec<Value>>>),
    Object(Rc<RefCell<HashMap<Arc<str>, Value>>>),
    /// User-defined function with AST body
    Function {
        params: Arc<[Arc<str>]>,
        defaults: Arc<[Option<Expr>]>,
        rest_param: Option<Arc<str>>,
        body: Arc<Statement>,
    },
    /// Native/builtin function
    Native(NativeFn),
    /// HTTP serve function (needs special handling for callbacks)
    #[cfg(feature = "http")]
    Serve,
    #[cfg(feature = "regex")]
    RegExp(Rc<RefCell<TishRegExp>>),
    /// Promise (ECMA-262 §27.2). Requires http feature for tokio.
    #[cfg(feature = "http")]
    Promise(crate::promise::PromiseRef),
    /// Internal: resolve/reject functions passed to executor. Not user-facing.
    #[cfg(feature = "http")]
    PromiseResolver(PromiseResolver),
    /// Promise constructor: Promise(executor). Requires special call handling.
    #[cfg(feature = "http")]
    PromiseConstructor,
    /// Bound promise method: promise.then/catch/finally - captures the promise for the call.
    #[cfg(feature = "http")]
    BoundPromiseMethod(crate::promise::PromiseRef, std::sync::Arc<str>),
    /// Timer builtins: setTimeout, setInterval. Need evaluator for callback.
    #[cfg(feature = "http")]
    TimerBuiltin(std::sync::Arc<str>),
    /// Native `tish_core` Promise (fetch / reader.read / response.text).
    #[cfg(feature = "http")]
    CorePromise(Arc<dyn TishPromise>),
    /// `tish_core::Value::Function` (e.g. response.text/json) callable from eval.
    #[cfg(feature = "http")]
    CoreFn(CoreNativeFn),
    /// Opaque handle to a native Rust type (e.g. Polars DataFrame).
    Opaque(Arc<dyn TishOpaque>),
    /// Bound method on an opaque value (opaque, method_name). Callable.
    OpaqueMethod(Arc<dyn TishOpaque>, Arc<str>),
}

impl std::fmt::Debug for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Number(n) => write!(f, "Number({})", n),
            Value::String(s) => write!(f, "String({:?})", s.as_ref()),
            Value::Bool(b) => write!(f, "Bool({})", b),
            Value::Null => write!(f, "Null"),
            Value::Array(arr) => write!(f, "Array({:?})", arr.borrow()),
            Value::Object(obj) => write!(f, "Object({:?})", obj.borrow()),
            Value::Function { .. } => write!(f, "Function"),
            Value::Native(_) => write!(f, "Native"),
            #[cfg(feature = "http")]
            Value::Serve => write!(f, "Serve"),
            #[cfg(feature = "regex")]
            Value::RegExp(re) => write!(f, "RegExp(/{}/{})", re.borrow().source, re.borrow().flags_string()),
            #[cfg(feature = "http")]
            Value::Promise(_) => write!(f, "Promise"),
            #[cfg(feature = "http")]
            Value::PromiseResolver(_) => write!(f, "[PromiseResolver]"),
            #[cfg(feature = "http")]
            Value::PromiseConstructor => write!(f, "[Function: Promise]"),
            #[cfg(feature = "http")]
            Value::BoundPromiseMethod(_, _) | Value::TimerBuiltin(_) => write!(f, "[Function]"),
            #[cfg(feature = "http")]
            Value::CorePromise(_) => write!(f, "Promise"),
            #[cfg(feature = "http")]
            Value::CoreFn(_) => write!(f, "CoreFn"),
            Value::Opaque(o) => write!(f, "{}(opaque)", o.type_name()),
            Value::OpaqueMethod(_, _) => write!(f, "[Function]"),
        }
    }
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
            Value::Native(_) => write!(f, "[NativeFunction]"),
            #[cfg(feature = "http")]
            Value::Serve => write!(f, "[NativeFunction: serve]"),
            #[cfg(feature = "regex")]
            Value::RegExp(re) => {
                let re = re.borrow();
                write!(f, "/{}/{}", re.source, re.flags_string())
            }
            #[cfg(feature = "http")]
            Value::Promise(_) => write!(f, "[Promise]"),
            #[cfg(feature = "http")]
            Value::PromiseResolver(_) => write!(f, "[Function]"),
            #[cfg(feature = "http")]
            Value::PromiseConstructor => write!(f, "function Promise() {{ [native code] }}"),
            #[cfg(feature = "http")]
            Value::BoundPromiseMethod(_, _) | Value::TimerBuiltin(_) => write!(f, "[Function]"),
            #[cfg(feature = "http")]
            Value::CorePromise(_) => write!(f, "[Promise]"),
            #[cfg(feature = "http")]
            Value::CoreFn(_) => write!(f, "[Function]"),
            Value::Opaque(o) => write!(f, "[object {}]", o.type_name()),
            Value::OpaqueMethod(_, _) => write!(f, "[Function]"),
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
            (Value::Number(a), Value::Number(b)) => {
                if a.is_nan() || b.is_nan() {
                    false
                } else {
                    a == b
                }
            }
            (Value::String(a), Value::String(b)) => a == b,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Null, Value::Null) => true,
            (Value::Array(a), Value::Array(b)) => Rc::ptr_eq(a, b),
            (Value::Object(a), Value::Object(b)) => Rc::ptr_eq(a, b),
            (Value::Opaque(a), Value::Opaque(b)) => Arc::ptr_eq(a, b),
            (Value::OpaqueMethod(a, ak), Value::OpaqueMethod(b, bk)) => Arc::ptr_eq(a, b) && ak == bk,
            _ => false,
        }
    }

    /// Create a new array Value from a Vec.
    pub fn array(items: Vec<Value>) -> Self {
        Value::Array(Rc::new(RefCell::new(items)))
    }

    /// Create a new object Value from a HashMap.
    pub fn object(map: HashMap<Arc<str>, Value>) -> Self {
        Value::Object(Rc::new(RefCell::new(map)))
    }

    /// Create an empty array Value.
    pub fn empty_array() -> Self {
        Value::Array(Rc::new(RefCell::new(Vec::new())))
    }

    /// Create an empty object Value.
    pub fn empty_object() -> Self {
        Value::Object(Rc::new(RefCell::new(HashMap::new())))
    }

    /// Extract the number value, if this is a Number.
    pub fn as_number(&self) -> Option<f64> {
        match self {
            Value::Number(n) => Some(*n),
            _ => None,
        }
    }
}
