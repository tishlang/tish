//! Runtime values for the Tish interpreter.
//!
//! This module defines the interpreter's `Value` type, which includes variants
//! like `Function`, `Native`, and `Serve` that hold AST or interpreter-specific
//! data. The compiled runtime uses `tishlang_core::Value` instead, which has a
//! different shape (no AST-carrying variants). The split is intentional.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use ahash::AHashMap;
use tishlang_ast::{FunParam, Statement};
use tishlang_core::NativeFn as CoreNativeFn;
use tishlang_core::TishSymbol;

/// Property map for interpreter object string keys (uses `eval::Value`, not `tishlang_core::Value`).
/// `IndexMap` preserves insertion order so `Object.keys` / `JSON.stringify` match JS/Node
/// (and the VM/rust backends, which use `tishlang_core`'s insertion-ordered `PropMap`).
pub type PropMap = indexmap::IndexMap<Arc<str>, Value>;

/// Interpreter object: string keys plus optional symbol-keyed properties.
#[derive(Clone, Debug, Default)]
pub struct EvalObjectData {
    pub strings: PropMap,
    pub symbols: Option<AHashMap<u64, Value>>,
    /// `Object.freeze` marker (#437) — writes to a frozen object throw a catchable TypeError.
    pub frozen: bool,
}

impl EvalObjectData {
    pub fn from_strings(strings: PropMap) -> Self {
        Self {
            strings,
            symbols: None,
            frozen: false,
        }
    }

    #[inline]
    pub fn is_frozen(&self) -> bool {
        self.frozen
    }
}

pub fn eval_object_get(obj: &Value, key: &Value) -> Option<Value> {
    let Value::Object(od) = obj else {
        return None;
    };
    let b = od.borrow();
    match key {
        Value::Symbol(s) => b.symbols.as_ref()?.get(&s.id).cloned(),
        Value::Number(n) => {
            let k: Arc<str> = n.to_string().into();
            b.strings.get(&k).cloned()
        }
        Value::String(k) => b.strings.get(k.as_ref()).cloned(),
        _ => None,
    }
}

pub fn eval_object_set(obj: &Value, key: &Value, val: Value) -> Result<(), String> {
    let Value::Object(od) = obj else {
        return Err("Cannot set property on non-object".to_string());
    };
    let mut b = od.borrow_mut();
    match key {
        Value::Symbol(s) => {
            if b.symbols.is_none() {
                b.symbols = Some(AHashMap::default());
            }
            b.symbols.as_mut().unwrap().insert(s.id, val);
            Ok(())
        }
        Value::Number(n) => {
            b.strings.insert(n.to_string().into(), val);
            Ok(())
        }
        Value::String(k) => {
            b.strings.insert(Arc::clone(k), val);
            Ok(())
        }
        _ => Err("Object key must be string, number, or symbol".to_string()),
    }
}

pub fn eval_object_has(obj: &Value, key: &Value) -> bool {
    let Value::Object(od) = obj else {
        return false;
    };
    let b = od.borrow();
    match key {
        Value::Symbol(s) => b.symbols.as_ref().is_some_and(|m| m.contains_key(&s.id)),
        Value::Number(n) => {
            let k: Arc<str> = n.to_string().into();
            b.strings.contains_key(&k)
        }
        Value::String(k) => b.strings.contains_key(k.as_ref()),
        _ => false,
    }
}
use tishlang_core::TishOpaque;
#[cfg(feature = "http")]
use tishlang_core::TishPromise;

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
    Object(Rc<RefCell<EvalObjectData>>),
    Symbol(Arc<TishSymbol>),
    /// User-defined function with AST body. `env` is the lexical scope captured at definition
    /// time, so the body resolves free variables against where it was DEFINED (a real closure),
    /// not where it is called.
    Function {
        formals: Arc<[FunParam]>,
        rest_param: Option<Arc<str>>,
        body: Arc<Statement>,
        env: crate::eval::ScopeRef,
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
    #[cfg(feature = "timers")]
    TimerBuiltin(std::sync::Arc<str>),
    /// Native `tishlang_core` Promise (fetch / reader.read / response.text).
    #[cfg(feature = "http")]
    CorePromise(Arc<dyn TishPromise>),
    /// `tishlang_core::Value::Function` (native callbacks, `new` constructors, fetch/ws when enabled).
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
            Value::RegExp(re) => write!(
                f,
                "RegExp(/{}/{})",
                re.borrow().source,
                re.borrow().flags_string()
            ),
            #[cfg(feature = "http")]
            Value::Promise(_) => write!(f, "Promise"),
            #[cfg(feature = "http")]
            Value::PromiseResolver(_) => write!(f, "[PromiseResolver]"),
            #[cfg(feature = "http")]
            Value::PromiseConstructor => write!(f, "[Function: Promise]"),
            #[cfg(feature = "http")]
            Value::BoundPromiseMethod(_, _) => write!(f, "[Function]"),
            #[cfg(feature = "timers")]
            Value::TimerBuiltin(_) => write!(f, "[Function]"),
            #[cfg(feature = "http")]
            Value::CorePromise(_) => write!(f, "Promise"),
            Value::CoreFn(_) => write!(f, "CoreFn"),
            Value::Opaque(o) => write!(f, "{}(opaque)", o.type_name()),
            Value::OpaqueMethod(_, _) => write!(f, "[Function]"),
            Value::Symbol(s) => write!(f, "Symbol({})", s.id),
        }
    }
}

impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            // Inspect form (`console.log`): keeps the sign of negative zero, unlike the ECMAScript
            // ToString used by `to_js_string`. Matches Node's console output (`console.log(-0)` →
            // `-0`). `to_js_string` has an explicit `Number` arm so ToString still drops it. (#247)
            Value::Number(n) if *n == 0.0 && n.is_sign_negative() => write!(f, "-0"),
            // Match JS `Number.prototype.toString` (exponential past digit 21 / before −6),
            // shared with the VM/native path via `tishlang_core`.
            Value::Number(n) => write!(f, "{}", tishlang_core::js_number_to_string(*n)),
            Value::String(s) => write!(f, "{}", s),
            Value::Bool(b) => write!(f, "{}", b),
            Value::Null => write!(f, "null"),
            // #381 — containers render through the cycle-guarded walker: a self-referential
            // array/object (`a.self = a`) previously recursed forever here (uncatchable native
            // stack overflow from any `console.log`/interpolation). Mirrors
            // `tishlang_core::Value::to_display_string_guarded` so interp output (`[Circular]`)
            // matches the VM/native backends.
            Value::Array(_) | Value::Object(_) => {
                write!(f, "{}", self.to_display_string_guarded(&mut Vec::new()))
            }
            Value::Symbol(s) => {
                if let Some(d) = &s.description {
                    write!(f, "Symbol({})", d)
                } else {
                    write!(f, "Symbol()")
                }
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
            Value::BoundPromiseMethod(_, _) => write!(f, "[Function]"),
            #[cfg(feature = "timers")]
            Value::TimerBuiltin(_) => write!(f, "[Function]"),
            #[cfg(feature = "http")]
            Value::CorePromise(_) => write!(f, "[Promise]"),
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

    /// JavaScript `ToString` coercion (as used by `Array.prototype.join`), distinct from the
    /// `Display`/inspect form: a nested **array** stringifies to its own comma-joined `toString`
    /// (recursively, always `,`), an **object** becomes `"[object Object]"`, and `null`/`undefined`
    /// elements elide to `""`. Mirrors `tishlang_core::Value::to_js_string` so interp output matches
    /// the VM/rust/cranelift/wasi backends (and Node) for join/coercion.
    pub fn to_js_string(&self) -> String {
        self.to_js_string_guarded(&mut Vec::new())
    }

    /// Cycle-safe inspect walker (#381): `ancestors` holds the current path's container pointers, so
    /// a self-referential array/object renders `[Circular]` (matching the VM/native backends via
    /// `tishlang_core::Value::to_display_string_guarded`) instead of recursing forever. Non-container
    /// leaves defer to `Display`, whose container arms route back here — with a fresh path — only
    /// for values this match already proved are not containers, so the guard cannot be bypassed.
    fn to_display_string_guarded(&self, ancestors: &mut Vec<*const ()>) -> String {
        match self {
            Value::Array(arr) => {
                let ptr = Rc::as_ptr(arr) as *const ();
                if ancestors.contains(&ptr) {
                    return "[Circular]".to_string();
                }
                ancestors.push(ptr);
                let inner: Vec<String> = arr
                    .borrow()
                    .iter()
                    .map(|v| v.to_display_string_guarded(ancestors))
                    .collect();
                ancestors.pop();
                format!("[{}]", inner.join(", "))
            }
            Value::Object(obj) => {
                let ptr = Rc::as_ptr(obj) as *const ();
                if ancestors.contains(&ptr) {
                    return "[Circular]".to_string();
                }
                ancestors.push(ptr);
                let inner: Vec<String> = obj
                    .borrow()
                    .strings
                    .iter()
                    .map(|(k, v)| {
                        format!("{}: {}", k.as_ref(), v.to_display_string_guarded(ancestors))
                    })
                    .collect();
                ancestors.pop();
                format!("{{{}}}", inner.join(", "))
            }
            other => other.to_string(),
        }
    }

    /// Cycle-safe `ToString` (#381): only arrays recurse here, so a cyclic array via `"" + a` would
    /// otherwise overflow the native stack (uncatchable abort). A back-reference joins as `""` —
    /// matching V8's `Array.prototype.join` and `tishlang_core::Value::to_js_string_guarded`, so
    /// interp coercion output matches the VM/native backends. Ancestor-path only: a node repeated
    /// across sibling branches is a legal DAG and still stringifies in full.
    fn to_js_string_guarded(&self, ancestors: &mut Vec<*const ()>) -> String {
        match self {
            Value::Array(arr) => {
                let ptr = Rc::as_ptr(arr) as *const ();
                if ancestors.contains(&ptr) {
                    return String::new();
                }
                ancestors.push(ptr);
                let s = arr
                    .borrow()
                    .iter()
                    .map(|v| match v {
                        Value::Null => String::new(),
                        other => other.to_js_string_guarded(ancestors),
                    })
                    .collect::<Vec<_>>()
                    .join(",");
                ancestors.pop();
                s
            }
            Value::Object(_) => "[object Object]".to_string(),
            // ECMAScript ToString of a number drops `-0`'s sign (`String(-0) === "0"`), distinct
            // from the inspect `Display` above which keeps it. Explicit arm so the `_` fallback to
            // `to_string()` (inspect) is not used for numbers. (#247)
            Value::Number(n) => tishlang_core::js_number_to_string(*n),
            _ => self.to_string(),
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
            (Value::Symbol(a), Value::Symbol(b)) => Arc::ptr_eq(a, b),
            (Value::Opaque(a), Value::Opaque(b)) => Arc::ptr_eq(a, b),
            (Value::OpaqueMethod(a, ak), Value::OpaqueMethod(b, bk)) => {
                Arc::ptr_eq(a, b) && ak == bk
            }
            _ => false,
        }
    }

    /// Create a new array Value from a Vec.
    pub fn array(items: Vec<Value>) -> Self {
        Value::Array(Rc::new(RefCell::new(items)))
    }

    /// Create a new object Value from a property map.
    pub fn object(map: PropMap) -> Self {
        Value::Object(Rc::new(RefCell::new(EvalObjectData::from_strings(map))))
    }

    /// Create an empty array Value.
    pub fn empty_array() -> Self {
        Value::Array(Rc::new(RefCell::new(Vec::new())))
    }

    /// Create an empty object Value.
    pub fn empty_object() -> Self {
        Value::Object(Rc::new(RefCell::new(EvalObjectData::default())))
    }

    /// Extract the number value, if this is a Number.
    pub fn as_number(&self) -> Option<f64> {
        match self {
            Value::Number(n) => Some(*n),
            _ => None,
        }
    }
}
