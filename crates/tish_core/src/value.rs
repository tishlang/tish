//! Unified Value type for Tish runtime values.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use ahash::AHashMap;
use indexmap::IndexMap;
use smallvec::SmallVec;

use crate::vmref::VmRef;

/// Property map for objects and other `Arc<str>` → `Value` tables (VM globals, scopes).
/// Uses a faster hasher than `std::collections::HashMap` for string-heavy workloads.
pub type ObjectMap = AHashMap<Arc<str>, Value>;

static NEXT_SYMBOL_ID: AtomicU64 = AtomicU64::new(1);

fn next_symbol_id() -> u64 {
    NEXT_SYMBOL_ID.fetch_add(1, Ordering::Relaxed)
}

/// Allocate a unique symbol id (for `Symbol()` and first-time `Symbol.for` entries).
#[inline]
pub fn alloc_symbol_id() -> u64 {
    next_symbol_id()
}

/// Primitive Symbol (ECMAScript-style): identity is `Arc` pointer equality.
#[derive(Debug)]
pub struct TishSymbol {
    pub id: u64,
    pub description: Option<Arc<str>>,
    /// Set when created via `Symbol.for(key)` (global registry).
    pub registry_key: Option<Arc<str>>,
}

impl TishSymbol {
    /// Unique symbol (`Symbol("desc")`).
    pub fn new_unique(description: Option<Arc<str>>) -> Arc<Self> {
        Arc::new(Self {
            id: next_symbol_id(),
            description,
            registry_key: None,
        })
    }

    /// Registry symbol (`Symbol.for`): stable `id` for this registry key.
    pub fn new_registry(id: u64, registry_key: Arc<str>, description: Option<Arc<str>>) -> Arc<Self> {
        Arc::new(Self {
            id,
            description,
            registry_key: Some(registry_key),
        })
    }
}

#[cfg(feature = "regex")]
use fancy_regex::Regex;

/// Native function signature.
///
/// When the `send-values` feature is enabled this is
/// `Arc<dyn Fn + Send + Sync>`, so handler closures can be dispatched across
/// HTTP worker threads (`tishlang_runtime::http::serve`). Otherwise it stays
/// `Rc<dyn Fn>` for zero-overhead single-threaded execution (wasm / wasi /
/// interpreter / cranelift / llvm VMs and any Rust native build without
/// `http`).
#[cfg(feature = "send-values")]
pub type NativeFn = Arc<dyn Fn(&[Value]) -> Value + Send + Sync>;
#[cfg(not(feature = "send-values"))]
pub type NativeFn = std::rc::Rc<dyn Fn(&[Value]) -> Value>;

/// Trait for opaque Rust types exposed to Tish (e.g. Polars DataFrame).
/// Implementors provide method dispatch so Tish can call methods on the value.
///
/// The `Send + Sync` supertrait bound is conditional on the `send-values`
/// feature. When `send-values` is off (single-threaded VMs: wasm browser /
/// wasi / interpreter / cranelift), `NativeFn` is already `Rc<dyn Fn>`, so
/// `Value` is `!Send` anyway — dropping the bound here loses nothing and lets
/// `!Send` opaques like `JsHandle(wasm_bindgen::JsValue)` be stored in a
/// `Value::Opaque` on the browser runtime.
#[cfg(feature = "send-values")]
pub trait TishOpaque: Send + Sync {
    /// Display name for the type (e.g. "DataFrame").
    fn type_name(&self) -> &'static str;

    /// Get a method by name. Returns a native function if the method exists.
    fn get_method(&self, name: &str) -> Option<NativeFn>;

    /// For downcasting `Arc<dyn TishOpaque>` in native crates (e.g. Polars → `DataFrame`).
    fn as_any(&self) -> &dyn std::any::Any;
}

/// Single-threaded variant (no `Send + Sync` bound); see the `send-values` doc above.
#[cfg(not(feature = "send-values"))]
pub trait TishOpaque {
    /// Display name for the type (e.g. "DataFrame").
    fn type_name(&self) -> &'static str;

    /// Get a method by name. Returns a native function if the method exists.
    fn get_method(&self, name: &str) -> Option<NativeFn>;

    /// For downcasting `Arc<dyn TishOpaque>` in native crates (e.g. Polars → `DataFrame`).
    fn as_any(&self) -> &dyn std::any::Any;
}

/// Trait for Promise-like values that can be awaited (block until settled).
/// Implemented by the runtime for native compile; interpreter uses its own Promise.
pub trait TishPromise: Send + Sync {
    fn block_until_settled(&self) -> std::result::Result<Value, Value>;
}

/// JavaScript RegExp flags
#[cfg(feature = "regex")]
#[derive(Debug, Clone, Default)]
pub struct RegExpFlags {
    pub global: bool,
    pub ignore_case: bool,
    pub multiline: bool,
    pub dot_all: bool,
    pub unicode: bool,
    pub sticky: bool,
}

#[cfg(feature = "regex")]
impl RegExpFlags {
    pub fn from_string(flags: &str) -> Result<Self, String> {
        let mut result = Self::default();
        for c in flags.chars() {
            match c {
                'g' => {
                    if result.global {
                        return Err(format!("duplicate flag '{}'", c));
                    }
                    result.global = true;
                }
                'i' => {
                    if result.ignore_case {
                        return Err(format!("duplicate flag '{}'", c));
                    }
                    result.ignore_case = true;
                }
                'm' => {
                    if result.multiline {
                        return Err(format!("duplicate flag '{}'", c));
                    }
                    result.multiline = true;
                }
                's' => {
                    if result.dot_all {
                        return Err(format!("duplicate flag '{}'", c));
                    }
                    result.dot_all = true;
                }
                'u' => {
                    if result.unicode {
                        return Err(format!("duplicate flag '{}'", c));
                    }
                    result.unicode = true;
                }
                'y' => {
                    if result.sticky {
                        return Err(format!("duplicate flag '{}'", c));
                    }
                    result.sticky = true;
                }
                _ => return Err(format!("unknown flag '{}'", c)),
            }
        }
        Ok(result)
    }
}

#[cfg(feature = "regex")]
impl std::fmt::Display for RegExpFlags {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.global {
            f.write_str("g")?;
        }
        if self.ignore_case {
            f.write_str("i")?;
        }
        if self.multiline {
            f.write_str("m")?;
        }
        if self.dot_all {
            f.write_str("s")?;
        }
        if self.unicode {
            f.write_str("u")?;
        }
        if self.sticky {
            f.write_str("y")?;
        }
        Ok(())
    }
}

/// Tish RegExp object
#[cfg(feature = "regex")]
#[derive(Debug, Clone)]
pub struct TishRegExp {
    pub source: String,
    pub flags: RegExpFlags,
    pub regex: Arc<Regex>,
    pub last_index: usize,
}

#[cfg(feature = "regex")]
impl TishRegExp {
    pub fn new(pattern: &str, flags_str: &str) -> Result<Self, String> {
        let flags = RegExpFlags::from_string(flags_str)?;
        let mut regex_pattern = pattern.to_string();

        if flags.ignore_case || flags.multiline || flags.dot_all {
            let mut flag_prefix = String::from("(?");
            if flags.ignore_case {
                flag_prefix.push('i');
            }
            if flags.multiline {
                flag_prefix.push('m');
            }
            if flags.dot_all {
                flag_prefix.push('s');
            }
            flag_prefix.push(')');
            regex_pattern = format!("{}{}", flag_prefix, regex_pattern);
        }

        let regex =
            Regex::new(&regex_pattern).map_err(|e| format!("Invalid regular expression: {}", e))?;

        Ok(Self {
            source: pattern.to_string(),
            flags,
            regex: Arc::new(regex),
            last_index: 0,
        })
    }

    pub fn flags_string(&self) -> String {
        self.flags.to_string()
    }

    pub fn test(&mut self, input: &str) -> bool {
        if self.flags.global || self.flags.sticky {
            let start = self.last_index;
            if start > input.chars().count() {
                self.last_index = 0;
                return false;
            }

            let byte_start: usize = input.chars().take(start).map(|c| c.len_utf8()).sum();
            let search_str = &input[byte_start..];

            match self.regex.find(search_str) {
                Ok(Some(m)) => {
                    if self.flags.sticky && m.start() != 0 {
                        self.last_index = 0;
                        return false;
                    }
                    let match_end_chars = input[byte_start..byte_start + m.end()].chars().count();
                    self.last_index = start + match_end_chars;
                    true
                }
                _ => {
                    self.last_index = 0;
                    false
                }
            }
        } else {
            self.regex.is_match(input).unwrap_or(false)
        }
    }
}

/// Runtime value for Tish programs.
/// Used by both interpreter and compiled code.
///
/// **Thread safety**: `Value: Send + Sync`. Mutable payloads live inside
/// [`VmRef`], a `Send + Sync` `Arc<Mutex<T>>` wrapper that preserves the
/// `RefCell`-style borrow API. Functions are `Arc<dyn Fn + Send + Sync>`.
#[derive(Clone)]
pub enum Value {
    Number(f64),
    String(Arc<str>),
    Bool(bool),
    Null,
    Array(VmRef<Vec<Value>>),
    Object(VmRef<ObjectData>),
    /// ECMAScript-style primitive symbol (identity by `Arc`).
    Symbol(Arc<TishSymbol>),
    Function(NativeFn),
    #[cfg(feature = "regex")]
    RegExp(VmRef<TishRegExp>),
    /// Promise (for native compile). Interpreter uses tishlang_eval::Value::Promise.
    Promise(Arc<dyn TishPromise>),
    /// Opaque handle to a native Rust type (e.g. Polars DataFrame).
    Opaque(Arc<dyn TishOpaque>),
}

/// Number of properties kept inline (no heap hashmap) before promoting to a map.
const PROPMAP_INLINE: usize = 8;

/// String-keyed property storage for objects.
///
/// Small objects (the overwhelming common case — `{ id, name, active }`) keep
/// their entries inline with linear-scan lookup: no separate hashmap allocation
/// and good cache locality, which beats hashing for a handful of keys. Objects
/// that grow past [`PROPMAP_INLINE`] keys promote to an insertion-ordered
/// `IndexMap` so large objects (e.g. `JSON.parse` output) keep O(1) lookup and
/// never hit O(n²). Iteration is always **insertion order**, matching JS/Node.
///
/// Exposes the `AHashMap`-compatible surface (`get`/`insert`/`iter`/…) the rest
/// of the runtime already uses, so it is a drop-in for the old `ObjectMap` field.
#[derive(Clone, Debug, Default)]
pub struct PropMap {
    inline: SmallVec<[(Arc<str>, Value); PROPMAP_INLINE]>,
    map: Option<Box<IndexMap<Arc<str>, Value, ahash::RandomState>>>,
}

impl PropMap {
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_capacity(n: usize) -> Self {
        if n > PROPMAP_INLINE {
            Self {
                inline: SmallVec::new(),
                map: Some(Box::new(IndexMap::with_capacity_and_hasher(
                    n,
                    ahash::RandomState::default(),
                ))),
            }
        } else {
            Self::default()
        }
    }

    #[inline]
    pub fn len(&self) -> usize {
        match &self.map {
            Some(m) => m.len(),
            None => self.inline.len(),
        }
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[inline]
    pub fn get(&self, key: &str) -> Option<&Value> {
        match &self.map {
            Some(m) => m.get(key),
            None => self
                .inline
                .iter()
                .find(|(k, _)| k.as_ref() == key)
                .map(|(_, v)| v),
        }
    }

    #[inline]
    pub fn get_mut(&mut self, key: &str) -> Option<&mut Value> {
        match &mut self.map {
            Some(m) => m.get_mut(key),
            None => self
                .inline
                .iter_mut()
                .find(|(k, _)| k.as_ref() == key)
                .map(|(_, v)| v),
        }
    }

    #[inline]
    pub fn contains_key(&self, key: &str) -> bool {
        self.get(key).is_some()
    }

    pub fn insert(&mut self, key: Arc<str>, val: Value) -> Option<Value> {
        if let Some(m) = &mut self.map {
            return m.insert(key, val);
        }
        if let Some(slot) = self.inline.iter_mut().find(|(k, _)| k.as_ref() == key.as_ref()) {
            return Some(std::mem::replace(&mut slot.1, val));
        }
        if self.inline.len() >= PROPMAP_INLINE {
            // Promote inline storage to an insertion-ordered map.
            let mut m: IndexMap<Arc<str>, Value, ahash::RandomState> =
                IndexMap::with_capacity_and_hasher(self.inline.len() + 1, ahash::RandomState::default());
            for (k, v) in self.inline.drain(..) {
                m.insert(k, v);
            }
            m.insert(key, val);
            self.map = Some(Box::new(m));
            return None;
        }
        self.inline.push((key, val));
        None
    }

    pub fn remove(&mut self, key: &str) -> Option<Value> {
        match &mut self.map {
            // shift_remove preserves insertion order (vs swap_remove).
            Some(m) => m.shift_remove(key),
            None => self
                .inline
                .iter()
                .position(|(k, _)| k.as_ref() == key)
                .map(|pos| self.inline.remove(pos).1),
        }
    }

    // Iterators return concrete enum types (not `Box<dyn>`) so iteration never
    // heap-allocates — critical for the per-request JSON stringify hot path
    // (`json.rs` iterates `strings.keys()` on every response object).
    #[inline]
    pub fn iter(&self) -> PropMapIter<'_> {
        match &self.map {
            Some(m) => PropMapIter::Map(m.iter()),
            None => PropMapIter::Inline(self.inline.iter()),
        }
    }

    #[inline]
    pub fn keys(&self) -> PropMapKeys<'_> {
        match &self.map {
            Some(m) => PropMapKeys::Map(m.keys()),
            None => PropMapKeys::Inline(self.inline.iter()),
        }
    }

    #[inline]
    pub fn values(&self) -> PropMapValues<'_> {
        match &self.map {
            Some(m) => PropMapValues::Map(m.values()),
            None => PropMapValues::Inline(self.inline.iter()),
        }
    }

    pub fn reserve(&mut self, additional: usize) {
        if let Some(m) = &mut self.map {
            m.reserve(additional);
        }
    }
}

impl FromIterator<(Arc<str>, Value)> for PropMap {
    fn from_iter<I: IntoIterator<Item = (Arc<str>, Value)>>(iter: I) -> Self {
        let mut pm = PropMap::default();
        for (k, v) in iter {
            pm.insert(k, v);
        }
        pm
    }
}

impl Extend<(Arc<str>, Value)> for PropMap {
    fn extend<I: IntoIterator<Item = (Arc<str>, Value)>>(&mut self, iter: I) {
        for (k, v) in iter {
            self.insert(k, v);
        }
    }
}

impl IntoIterator for PropMap {
    type Item = (Arc<str>, Value);
    type IntoIter = PropMapIntoIter;
    fn into_iter(self) -> Self::IntoIter {
        match self.map {
            Some(m) => PropMapIntoIter::Map(m.into_iter()),
            None => PropMapIntoIter::Inline(self.inline.into_iter()),
        }
    }
}

/// Zero-allocation borrowing iterator over [`PropMap`] entries (insertion order).
pub enum PropMapIter<'a> {
    Inline(std::slice::Iter<'a, (Arc<str>, Value)>),
    Map(indexmap::map::Iter<'a, Arc<str>, Value>),
}
impl<'a> Iterator for PropMapIter<'a> {
    type Item = (&'a Arc<str>, &'a Value);
    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        match self {
            PropMapIter::Inline(it) => it.next().map(|(k, v)| (k, v)),
            PropMapIter::Map(it) => it.next(),
        }
    }
}

/// Zero-allocation key iterator over [`PropMap`] (insertion order).
pub enum PropMapKeys<'a> {
    Inline(std::slice::Iter<'a, (Arc<str>, Value)>),
    Map(indexmap::map::Keys<'a, Arc<str>, Value>),
}
impl<'a> Iterator for PropMapKeys<'a> {
    type Item = &'a Arc<str>;
    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        match self {
            PropMapKeys::Inline(it) => it.next().map(|(k, _)| k),
            PropMapKeys::Map(it) => it.next(),
        }
    }
}

/// Zero-allocation value iterator over [`PropMap`] (insertion order).
pub enum PropMapValues<'a> {
    Inline(std::slice::Iter<'a, (Arc<str>, Value)>),
    Map(indexmap::map::Values<'a, Arc<str>, Value>),
}
impl<'a> Iterator for PropMapValues<'a> {
    type Item = &'a Value;
    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        match self {
            PropMapValues::Inline(it) => it.next().map(|(_, v)| v),
            PropMapValues::Map(it) => it.next(),
        }
    }
}

/// Owning iterator over [`PropMap`] entries (insertion order).
pub enum PropMapIntoIter {
    Inline(smallvec::IntoIter<[(Arc<str>, Value); PROPMAP_INLINE]>),
    Map(indexmap::map::IntoIter<Arc<str>, Value>),
}
impl Iterator for PropMapIntoIter {
    type Item = (Arc<str>, Value);
    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        match self {
            PropMapIntoIter::Inline(it) => it.next(),
            PropMapIntoIter::Map(it) => it.next(),
        }
    }
}

/// Ordinary object: string-keyed properties plus optional symbol-keyed side map.
#[derive(Clone, Debug, Default)]
pub struct ObjectData {
    pub strings: PropMap,
    pub symbols: Option<AHashMap<u64, Value>>,
}

impl ObjectData {
    #[inline]
    pub fn from_strings<I: IntoIterator<Item = (Arc<str>, Value)>>(strings: I) -> Self {
        Self {
            strings: strings.into_iter().collect(),
            symbols: None,
        }
    }

    #[inline]
    pub fn len_entries(&self) -> usize {
        self.strings.len() + self.symbols.as_ref().map(|s| s.len()).unwrap_or(0)
    }
}

/// Read a property from an object value.
pub fn object_get(obj: &Value, key: &Value) -> Option<Value> {
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

/// Set a property on an object.
pub fn object_set(obj: &Value, key: &Value, val: Value) -> Result<(), String> {
    let Value::Object(od) = obj else {
        return Err(format!("Cannot set property on {}", obj.type_name()));
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
        _ => Err(format!(
            "Object key must be string, number, or symbol, got {}",
            key.type_name()
        )),
    }
}

/// `key in obj` for objects.
pub fn object_has(obj: &Value, key: &Value) -> bool {
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

/// Invoke a callable [`Value`]: [`Value::Function`], or an object exposing `__call` (e.g. `Symbol`).
pub fn value_call(callee: &Value, args: &[Value]) -> Value {
    match callee {
        Value::Function(f) => f(args),
        Value::Object(o) => {
            let inner = o.borrow().strings.get("__call").cloned();
            if let Some(inner) = inner {
                return value_call(&inner, args);
            }
            panic!(
                "Not a function: tried to call {:?} as a function (e.g. method on Null when read failed)",
                callee
            );
        }
        _ => panic!(
            "Not a function: tried to call {:?} as a function (e.g. method on Null when read failed)",
            callee
        ),
    }
}

/// Merge two object payloads (spread / VM MergeObject).
pub fn merge_object_data(left: &VmRef<ObjectData>, right: &VmRef<ObjectData>) -> ObjectData {
    let l = left.borrow();
    let r = right.borrow();
    let mut strings = PropMap::with_capacity(l.strings.len() + r.strings.len());
    strings.extend(l.strings.iter().map(|(k, v)| (Arc::clone(k), v.clone())));
    strings.extend(r.strings.iter().map(|(k, v)| (Arc::clone(k), v.clone())));
    let mut symbols: Option<AHashMap<u64, Value>> = None;
    if let Some(ls) = &l.symbols {
        symbols = Some(ls.clone());
    }
    if let Some(rs) = &r.symbols {
        match &mut symbols {
            Some(m) => {
                m.extend(rs.iter().map(|(k, v)| (*k, v.clone())));
            }
            None => symbols = Some(rs.clone()),
        }
    }
    ObjectData { strings, symbols }
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
            Value::Symbol(s) => write!(f, "Symbol({})", s.id),
            Value::Function(_) => write!(f, "Function"),
            #[cfg(feature = "regex")]
            Value::RegExp(re) => write!(
                f,
                "RegExp(/{}/{})",
                re.borrow().source,
                re.borrow().flags_string()
            ),
            Value::Promise(_) => write!(f, "Promise"),
            Value::Opaque(o) => write!(f, "{}(opaque)", o.type_name()),
        }
    }
}

impl Value {
    /// Convert value to display string (for console output).
    pub fn to_display_string(&self) -> String {
        match self {
            Value::Number(n) => {
                if n.is_nan() {
                    "NaN".to_string()
                } else if *n == f64::INFINITY {
                    "Infinity".to_string()
                } else if *n == f64::NEG_INFINITY {
                    "-Infinity".to_string()
                } else {
                    n.to_string()
                }
            }
            Value::String(s) => s.to_string(),
            Value::Bool(b) => b.to_string(),
            Value::Null => "null".to_string(),
            Value::Array(arr) => {
                let inner: Vec<String> =
                    arr.borrow().iter().map(|v| v.to_display_string()).collect();
                format!("[{}]", inner.join(", "))
            }
            Value::Object(obj) => {
                let inner: Vec<String> = obj
                    .borrow()
                    .strings
                    .iter()
                    .map(|(k, v)| format!("{}: {}", k.as_ref(), v.to_display_string()))
                    .collect();
                format!("{{{}}}", inner.join(", "))
            }
            Value::Symbol(s) => {
                if let Some(d) = &s.description {
                    format!("Symbol({})", d)
                } else {
                    "Symbol()".to_string()
                }
            }
            Value::Function(_) => "[Function]".to_string(),
            Value::Promise(_) => "[object Promise]".to_string(),
            Value::Opaque(o) => format!("[object {}]", o.type_name()),
            #[cfg(feature = "regex")]
            Value::RegExp(re) => {
                let re = re.borrow();
                format!("/{}/{}", re.source, re.flags_string())
            }
        }
    }

    /// Check if value is truthy (for conditionals).
    pub fn is_truthy(&self) -> bool {
        match self {
            Value::Null => false,
            Value::Bool(b) => *b,
            Value::Number(n) => *n != 0.0 && !n.is_nan(),
            Value::String(s) => !s.is_empty(),
            _ => true,
        }
    }

    /// Strict equality (===).
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
            (Value::Array(a), Value::Array(b)) => VmRef::ptr_eq(a, b),
            (Value::Object(a), Value::Object(b)) => VmRef::ptr_eq(a, b),
            #[cfg(feature = "send-values")]
            (Value::Function(a), Value::Function(b)) => Arc::ptr_eq(a, b),
            #[cfg(not(feature = "send-values"))]
            (Value::Function(a), Value::Function(b)) => std::rc::Rc::ptr_eq(a, b),
            #[cfg(feature = "regex")]
            (Value::RegExp(a), Value::RegExp(b)) => VmRef::ptr_eq(a, b),
            (Value::Promise(a), Value::Promise(b)) => Arc::ptr_eq(a, b),
            (Value::Opaque(a), Value::Opaque(b)) => Arc::ptr_eq(a, b),
            (Value::Symbol(a), Value::Symbol(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }

    /// Wrap a Rust closure in a `Value::Function`. Automatically picks
    /// `Rc<dyn Fn>` or `Arc<dyn Fn + Send + Sync>` based on the
    /// `send-values` feature, so callers don't have to `cfg`-gate their
    /// code. The input bound tracks the feature too: when `send-values`
    /// is enabled the closure must be `Send + Sync`, otherwise any `Fn`
    /// is accepted.
    #[cfg(feature = "send-values")]
    pub fn native<F>(f: F) -> Self
    where
        F: Fn(&[Value]) -> Value + Send + Sync + 'static,
    {
        Value::Function(Arc::new(f))
    }

    #[cfg(not(feature = "send-values"))]
    pub fn native<F>(f: F) -> Self
    where
        F: Fn(&[Value]) -> Value + 'static,
    {
        Value::Function(std::rc::Rc::new(f))
    }

    /// Create a new array Value from a Vec.
    pub fn array(items: Vec<Value>) -> Self {
        Value::Array(VmRef::new(items))
    }

    /// Create a new object Value from a property map.
    pub fn object(map: ObjectMap) -> Self {
        Value::Object(VmRef::new(ObjectData::from_strings(map)))
    }

    /// Create an empty array Value.
    pub fn empty_array() -> Self {
        Value::Array(VmRef::new(Vec::new()))
    }

    /// Create an empty object Value.
    pub fn empty_object() -> Self {
        Value::Object(VmRef::new(ObjectData::default()))
    }

    /// Extract the number value, if this is a Number.
    pub fn as_number(&self) -> Option<f64> {
        match self {
            Value::Number(n) => Some(*n),
            _ => None,
        }
    }

    /// JavaScript-style typeof string for this value.
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Number(_) => "number",
            Value::String(_) => "string",
            Value::Bool(_) => "boolean",
            Value::Null => "null",
            Value::Array(_) => "object",
            Value::Object(_) => "object",
            Value::Function(_) => "function",
            #[cfg(feature = "regex")]
            Value::RegExp(_) => "object",
            Value::Promise(_) => "object",
            Value::Opaque(o) => o.type_name(),
            Value::Symbol(_) => "symbol",
        }
    }

    /// Property/method names for REPL tab completion (e.g. after `obj.`).
    pub fn completion_keys(&self) -> Vec<String> {
        match self {
            Value::Object(m) => {
                let mut keys: Vec<String> = m
                    .borrow()
                    .strings
                    .keys()
                    .map(|k| k.to_string())
                    .collect();
                keys.sort();
                keys
            }
            Value::Array(_) => {
                vec![
                    "length".into(),
                    "at".into(),
                    "concat".into(),
                    "copyWithin".into(),
                    "entries".into(),
                    "every".into(),
                    "fill".into(),
                    "filter".into(),
                    "find".into(),
                    "findIndex".into(),
                    "findLast".into(),
                    "findLastIndex".into(),
                    "flat".into(),
                    "flatMap".into(),
                    "forEach".into(),
                    "includes".into(),
                    "indexOf".into(),
                    "join".into(),
                    "keys".into(),
                    "lastIndexOf".into(),
                    "map".into(),
                    "pop".into(),
                    "push".into(),
                    "reduce".into(),
                    "reduceRight".into(),
                    "reverse".into(),
                    "shift".into(),
                    "slice".into(),
                    "some".into(),
                    "sort".into(),
                    "splice".into(),
                    "toLocaleString".into(),
                    "toReversed".into(),
                    "toSorted".into(),
                    "toSpliced".into(),
                    "toString".into(),
                    "unshift".into(),
                    "values".into(),
                    "shuffle".into(),
                ]
            }
            Value::String(_) => {
                vec![
                    "length".into(),
                    "charAt".into(),
                    "charCodeAt".into(),
                    "endsWith".into(),
                    "includes".into(),
                    "indexOf".into(),
                    "lastIndexOf".into(),
                    "padEnd".into(),
                    "padStart".into(),
                    "repeat".into(),
                    "replace".into(),
                    "replaceAll".into(),
                    "slice".into(),
                    "split".into(),
                    "startsWith".into(),
                    "substring".into(),
                    "toLowerCase".into(),
                    "toUpperCase".into(),
                    "trim".into(),
                ]
            }
            Value::Number(_) => vec![
                "toFixed".into(),
                "toExponential".into(),
                "toPrecision".into(),
            ],
            _ => vec![],
        }
    }
}
