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
/// A callable value's behaviour. Replaces the former `Arc<dyn Fn(&[Value]) -> Value>`:
/// the trait lets a *bytecode-VM* closure additionally expose its compiled chunk (via the
/// `as_any` downcast), so the VM's `Call` opcode can run tish→tish calls on an explicit
/// frame stack (task #39, the frame-VM) instead of recursively re-entering `run_chunk` —
/// while native builtins use the blanket [`FnCallable`] adapter and keep plain `Fn`
/// behaviour. `Send + Sync` is conditional on `send-values`, exactly like `NativeFn` was.
#[cfg(feature = "send-values")]
pub trait Callable: Send + Sync {
    fn call(&self, args: &[Value]) -> Value;
    /// Downcast hook for the VM frame path; native adapters return themselves (downcast fails).
    fn as_any(&self) -> &dyn std::any::Any;
}
#[cfg(not(feature = "send-values"))]
pub trait Callable {
    fn call(&self, args: &[Value]) -> Value;
    fn as_any(&self) -> &dyn std::any::Any;
}

/// Adapter wrapping a plain `Fn` closure (every native builtin) as a [`Callable`].
pub struct FnCallable<F>(pub F);
#[cfg(feature = "send-values")]
impl<F: Fn(&[Value]) -> Value + Send + Sync + 'static> Callable for FnCallable<F> {
    #[inline]
    fn call(&self, args: &[Value]) -> Value {
        (self.0)(args)
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
#[cfg(not(feature = "send-values"))]
impl<F: Fn(&[Value]) -> Value + 'static> Callable for FnCallable<F> {
    #[inline]
    fn call(&self, args: &[Value]) -> Value {
        (self.0)(args)
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[cfg(feature = "send-values")]
pub type NativeFn = Arc<dyn Callable>;
#[cfg(not(feature = "send-values"))]
pub type NativeFn = std::rc::Rc<dyn Callable>;

/// Build a raw [`NativeFn`] from a plain closure (wraps it in [`FnCallable`]). For sites that
/// need a `NativeFn` handle directly rather than a `Value::Function` (e.g. HTTP/promise/timer
/// internals that store the callable). The `Value::Function` variant is built via [`Value::native`].
#[cfg(feature = "send-values")]
pub fn native_fn<F: Fn(&[Value]) -> Value + Send + Sync + 'static>(f: F) -> NativeFn {
    Arc::new(FnCallable(f))
}
#[cfg(not(feature = "send-values"))]
pub fn native_fn<F: Fn(&[Value]) -> Value + 'static>(f: F) -> NativeFn {
    std::rc::Rc::new(FnCallable(f))
}

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
    /// Try to settle WITHOUT blocking. Returns `Some(result)` if the promise was already
    /// settled before this call; returns `None` if it is still pending (a background thread
    /// / I/O task has not completed yet). Default: always pending — implementors of async
    /// promises (fetch, spawn) leave this as `None`; `ImmediateSettledPromise` overrides it.
    ///
    /// Used by `race`/`any`/`allSettled` to handle already-settled promises in input-order
    /// (deterministic, JS-compatible) before falling back to concurrent thread waiting for
    /// genuinely-pending ones.
    fn try_settle(&self) -> Option<std::result::Result<Value, Value>> {
        None
    }
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
#[derive(Clone, Default)]
pub enum Value {
    Number(f64),
    String(arcstr::ArcStr),
    Bool(bool),
    #[default]
    Null,
    Array(VmRef<Vec<Value>>),
    /// Packed f64 array — `TISH_PACKED_ARRAYS` mode only. All elements are f64; a non-numeric
    /// push/set/op materializes to `Value::Array` first. Eliminates per-element boxing and
    /// enables direct `sort_unstable_by` without an unbox pass. Created by all-numeric array
    /// literals, `new Array(n)` (zero-filled), and from numeric HOF results. Never created
    /// when `packed_arrays_enabled()` is false — callers check before constructing.
    NumberArray(VmRef<Vec<f64>>),
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

// Size guard. `Value` is 24 bytes: `String` is thin (`ArcStr`, 8B), but `Function`/`Promise`/`Opaque`
// are fat `Arc<dyn …>` (data+vtable, 16B) ⇒ 16B payload + discriminant = 24.
//
// NOTE — shrinking to 16B was tried and REVERTED (see docs/perf.md "Value-shrink"): thinning the
// three `Arc<dyn>` variants to `Arc<Box<dyn>>` (8B) DID make `Value` 16B and stayed green on all 6
// backends, but it REGRESSED numeric dispatch ~8–10% (measured A/B interleaved, both in- AND
// out-of-cache). tish is dispatch-bound, NOT memory-bandwidth-bound on `Value` size; the
// boxing/enum-layout change pessimized the hot `Number` path. Smaller `Value` ≠ faster here. Do not
// re-attempt the box trick; only a dispatch-level change (e.g. NaN-box's branch-free tag test, not
// its size) could pay off. Gated to 64-bit: wasm32 (wasi) has 32-bit pointers, so size differs there.
#[cfg(target_pointer_width = "64")]
const _: () = assert!(std::mem::size_of::<Value>() == 24);

// ─────────────────────────────────────────────────────────────────────────────
// #201 Stage A — representation abstraction (behavior-preserving, zero perf effect).
//
// `Value` is an enum today. To eventually swap it for a NaN-boxed `struct Value(u64)`
// (#201 Stage C) WITHOUT thousands of simultaneous edits, call sites should go through
// this abstraction instead of matching the enum directly:
//   * construct via the named constructors (`Value::number`, `::boolean`, `::string`, …),
//   * inspect via `v.unpack()` (a borrowed view) or the `as_*` / `tag` accessors.
// Once sites are migrated, the representation changes behind these method bodies and the
// call sites are untouched. Stage B (the 24→16 size-shrink) is intentionally SKIPPED — it
// was tried and regressed dispatch ~8-10% (see the size-guard note above); only Stage C's
// branch-free tag test can pay off. This layer is pure enabling: no behavior, no perf.
// ─────────────────────────────────────────────────────────────────────────────

/// Cheap type discriminant for a [`Value`]. `match v.tag()` will lower to a branch-free
/// tag test once `Value` is NaN-boxed (#201). Variant order/values are not part of any ABI.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ValueTag {
    Number,
    String,
    Bool,
    Null,
    Array,
    NumberArray,
    Object,
    Symbol,
    Function,
    #[cfg(feature = "regex")]
    RegExp,
    Promise,
    Opaque,
}

/// Borrowed view of a [`Value`]'s payload, mirroring the enum variants. Lets call sites
/// write `match v.unpack() { ValueRef::Number(n) => … }` instead of matching the enum
/// directly, so the underlying representation can change (#201) without touching the match
/// sites. Zero-cost — each arm borrows the existing payload in place.
pub enum ValueRef<'a> {
    Number(f64),
    String(&'a arcstr::ArcStr),
    Bool(bool),
    Null,
    Array(&'a VmRef<Vec<Value>>),
    NumberArray(&'a VmRef<Vec<f64>>),
    Object(&'a VmRef<ObjectData>),
    Symbol(&'a Arc<TishSymbol>),
    Function(&'a NativeFn),
    #[cfg(feature = "regex")]
    RegExp(&'a VmRef<TishRegExp>),
    Promise(&'a Arc<dyn TishPromise>),
    Opaque(&'a Arc<dyn TishOpaque>),
}

impl Value {
    /// Named constructor for a number (#201 abstraction). Prefer over `Value::Number(n)`.
    #[inline]
    pub fn number(n: f64) -> Self {
        Value::Number(n)
    }

    /// Named constructor for a boolean (#201 abstraction). Prefer over `Value::Bool(b)`.
    #[inline]
    pub fn boolean(b: bool) -> Self {
        Value::Bool(b)
    }

    /// Named constructor for a string (#201 abstraction). Accepts anything that converts
    /// into the interned `ArcStr` (`&str`, `String`, `ArcStr`).
    #[inline]
    pub fn string(s: impl Into<arcstr::ArcStr>) -> Self {
        Value::String(s.into())
    }

    /// Named constructor for null (#201 abstraction). Prefer over `Value::Null`.
    #[inline]
    pub fn null() -> Self {
        Value::Null
    }

    /// Borrowed view of the payload — the migration target for `match v { … }`.
    #[inline]
    pub fn unpack(&self) -> ValueRef<'_> {
        match self {
            Value::Number(n) => ValueRef::Number(*n),
            Value::String(s) => ValueRef::String(s),
            Value::Bool(b) => ValueRef::Bool(*b),
            Value::Null => ValueRef::Null,
            Value::Array(a) => ValueRef::Array(a),
            Value::NumberArray(a) => ValueRef::NumberArray(a),
            Value::Object(o) => ValueRef::Object(o),
            Value::Symbol(s) => ValueRef::Symbol(s),
            Value::Function(f) => ValueRef::Function(f),
            #[cfg(feature = "regex")]
            Value::RegExp(r) => ValueRef::RegExp(r),
            Value::Promise(p) => ValueRef::Promise(p),
            Value::Opaque(o) => ValueRef::Opaque(o),
        }
    }

    /// The value's type discriminant (#201 abstraction) — for cheap type checks.
    #[inline]
    pub fn tag(&self) -> ValueTag {
        match self {
            Value::Number(_) => ValueTag::Number,
            Value::String(_) => ValueTag::String,
            Value::Bool(_) => ValueTag::Bool,
            Value::Null => ValueTag::Null,
            Value::Array(_) => ValueTag::Array,
            Value::NumberArray(_) => ValueTag::NumberArray,
            Value::Object(_) => ValueTag::Object,
            Value::Symbol(_) => ValueTag::Symbol,
            Value::Function(_) => ValueTag::Function,
            #[cfg(feature = "regex")]
            Value::RegExp(_) => ValueTag::RegExp,
            Value::Promise(_) => ValueTag::Promise,
            Value::Opaque(_) => ValueTag::Opaque,
        }
    }

    /// Extract the boolean payload, if this is a `Bool`.
    #[inline]
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// Borrow the string payload as `&str`, if this is a `String`.
    #[inline]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s.as_str()),
            _ => None,
        }
    }

    /// Borrow the object payload, if this is an `Object`.
    #[inline]
    pub fn as_object(&self) -> Option<&VmRef<ObjectData>> {
        match self {
            Value::Object(o) => Some(o),
            _ => None,
        }
    }

    /// Borrow the array payload, if this is an `Array`.
    #[inline]
    pub fn as_array(&self) -> Option<&VmRef<Vec<Value>>> {
        match self {
            Value::Array(a) => Some(a),
            _ => None,
        }
    }

    /// Borrow the packed-number-array payload, if this is a `NumberArray`.
    #[inline]
    pub fn as_number_array(&self) -> Option<&VmRef<Vec<f64>>> {
        match self {
            Value::NumberArray(a) => Some(a),
            _ => None,
        }
    }

    /// True if this is `Null`.
    #[inline]
    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }
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
    /// Hidden-class identity for this object's ordered key-set (JSC Structure). `EMPTY_SHAPE` (0)
    /// for `{}`. Maintained by `insert` (new key → `shape::transition`) and reset to `DICT_SHAPE` by
    /// `remove`. Lets the VM's inline caches compare a `u32` instead of hashing a key. INVARIANT: a
    /// non-empty PropMap never has `EMPTY_SHAPE` (every key-add transitions away from it) — the IC
    /// relies on this, so all key-adds must go through `insert` (the only mutation path; fields private).
    shape: crate::shape::ShapeId,
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
                shape: crate::shape::EMPTY_SHAPE,
            }
        } else {
            Self::default()
        }
    }

    /// The hidden-class id for this object's current key-set (for the VM's inline caches).
    #[inline]
    pub fn shape(&self) -> crate::shape::ShapeId {
        self.shape
    }

    /// Value at slot `i` (insertion order). For the inline-cache hit path: once a `(shape, index)`
    /// is cached, a shape match means the property is at this stable index.
    #[inline]
    pub fn value_at_index(&self, i: usize) -> Option<&Value> {
        match &self.map {
            Some(m) => m.get_index(i).map(|(_, v)| v),
            None => self.inline.get(i).map(|(_, v)| v),
        }
    }

    /// Mutable value at slot `i` (insertion order) — for the SetMember inline-cache update path.
    #[inline]
    pub fn value_at_index_mut(&mut self, i: usize) -> Option<&mut Value> {
        match &mut self.map {
            Some(m) => m.get_index_mut(i).map(|(_, v)| v),
            None => self.inline.get_mut(i).map(|(_, v)| v),
        }
    }

    /// Like `get`, but also returns the property's slot index — used to *fill* an inline cache on a miss.
    #[inline]
    pub fn get_with_index(&self, key: &str) -> Option<(&Value, usize)> {
        match &self.map {
            Some(m) => m.get_full(key).map(|(i, _, v)| (v, i)),
            None => self
                .inline
                .iter()
                .position(|(k, _)| k.as_ref() == key)
                .map(|i| (&self.inline[i].1, i)),
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
            // Map path (>PROPMAP_INLINE keys). A new key transitions the shape; an update doesn't.
            let kc = Arc::clone(&key);
            let prev = m.insert(key, val);
            if prev.is_none() {
                self.shape = crate::shape::transition(self.shape, &kc);
            }
            return prev;
        }
        if let Some(slot) = self.inline.iter_mut().find(|(k, _)| k.as_ref() == key.as_ref()) {
            // Update existing key → value changes, layout (shape) does not.
            return Some(std::mem::replace(&mut slot.1, val));
        }
        // New key (inline storage) → transition the shape away from the current one.
        self.shape = crate::shape::transition(self.shape, &key);
        if self.inline.len() >= PROPMAP_INLINE {
            // Promote inline storage to an insertion-ordered map (keys + their order are preserved,
            // so the shape stays valid).
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
        let removed = match &mut self.map {
            // shift_remove preserves insertion order (vs swap_remove).
            Some(m) => m.shift_remove(key),
            None => self
                .inline
                .iter()
                .position(|(k, _)| k.as_ref() == key)
                .map(|pos| self.inline.remove(pos).1),
        };
        if removed.is_some() {
            // Deleting shifts slot indices → this object opts out of shape-based inline caches.
            self.shape = crate::shape::DICT_SHAPE;
        }
        removed
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

    /// Merge every entry of `other` into `self`, in `other`'s insertion order.
    ///
    /// Existing keys are overwritten (last write wins), so this preserves JS object-spread
    /// semantics: `{ ...a, ...b }` lets `b` override `a`. The work is pre-sized — if the merge
    /// would push the result past the inline threshold we promote to an `IndexMap` once, with the
    /// final capacity, instead of growing/rehashing one key at a time. The native codegen calls
    /// this for every `{ ...src }` spread, replacing the old "rebuild via AHashMap" path.
    pub fn merge_from(&mut self, other: &PropMap) {
        let incoming = other.len();
        if incoming == 0 {
            return;
        }
        // If the combined worst-case size escapes inline storage, promote once up front so the
        // per-key inserts below never reallocate or re-promote.
        if self.map.is_none() && self.inline.len() + incoming > PROPMAP_INLINE {
            let mut m: IndexMap<Arc<str>, Value, ahash::RandomState> = IndexMap::with_capacity_and_hasher(
                self.inline.len() + incoming,
                ahash::RandomState::default(),
            );
            for (k, v) in self.inline.drain(..) {
                m.insert(k, v);
            }
            self.map = Some(Box::new(m));
        } else if let Some(m) = &mut self.map {
            m.reserve(incoming);
        }
        for (k, v) in other.iter() {
            self.insert(Arc::clone(k), v.clone());
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
#[allow(clippy::large_enum_variant)] // `Inline` is intentionally unboxed to keep PropMap iteration allocation-free
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
            b.strings.insert(Arc::from(k.as_str()), val);
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

/// Drain a JS-style iterator object — one with a callable `next()` that returns
/// `{ value, done }` — into a `Vec`, calling `next()` until `done` is truthy. Returns
/// `None` when `obj` is not such an object (no callable `next`), so callers fall back
/// to their array/string handling. This is what makes a `Map`/`Set` iterator (the result
/// of `.values()`/`.keys()`/`.entries()`) usable in `for…of`, spread, and `Array.from`.
/// A missing/absent `done` is treated as truthy so a malformed object can't spin forever;
/// the native iterators always set `done`, so well-formed iteration is exact.
pub fn drain_iterator(obj: &Value) -> Option<Vec<Value>> {
    if !matches!(obj, Value::Object(_)) {
        return None;
    }
    // Fast path: tish's own `Map`/`Set` iterators expose `__drain__`, which returns the remaining
    // items as one array (respecting the current position) and exhausts the iterator — so `for…of`
    // and spread don't pay the per-element `{ value, done }` allocation of the generic `next()` loop.
    if let Some(Value::Function(drain)) = object_get(obj, &Value::String("__drain__".into())) {
        if let Value::Array(arr) = drain.call(&[]) {
            return Some(arr.borrow().clone());
        }
    }
    let Value::Function(next) = object_get(obj, &Value::String("next".into()))? else {
        return None;
    };
    let done_key = Value::String("done".into());
    let value_key = Value::String("value".into());
    let mut out = Vec::new();
    loop {
        let res = next.call(&[]);
        let done = object_get(&res, &done_key)
            .map(|v| v.is_truthy())
            .unwrap_or(true);
        if done {
            break;
        }
        out.push(object_get(&res, &value_key).unwrap_or(Value::Null));
    }
    Some(out)
}

/// JS `ToInt32`: NaN and ±Infinity map to `0`; every other value is truncated toward zero and
/// reduced modulo 2³². `f64 as i64` is exact for the finite `< 2⁶³` magnitudes real bitwise code
/// produces (then `as i32` truncates the low 32 bits = the modulo); the `is_finite` guard is what
/// makes `Infinity`/`-Infinity` correct — `f64 as i64` *saturates* (`+∞ → i64::MAX → -1`), which is
/// NOT the JS result. One always-predicted branch, so the hot path (finite hash values) is unaffected.
#[inline]
pub fn to_int32(x: f64) -> i32 {
    if x.is_finite() {
        x as i64 as i32
    } else {
        0
    }
}

/// JS `ToUint32`: as [`to_int32`] but reinterpreted unsigned (NaN/±Infinity → `0`).
#[inline]
pub fn to_uint32(x: f64) -> u32 {
    if x.is_finite() {
        x as i64 as u32
    } else {
        0
    }
}

/// `ToNumber` for the native boxed runtime: a `Value::Number` passes through, every other variant
/// coerces to `NaN`. This is the same `as_number().unwrap_or(NaN)` convention used by the
/// interpreter (`binop_number` / `to_int32`), the VM (`eval_binop`), and `ops::add`/`sub`/`mul`/`div`
/// — so the native backend stays bit-for-bit in lock-step with them at runtime (a string/bool/null
/// operand is `NaN`, hence `0` for bitwise). The compiler constant-folds literal cases like
/// `"5" | 0 === 5` separately, so this only governs runtime (non-constant) operands.
#[inline]
pub fn to_number_value(v: &Value) -> f64 {
    v.as_number().unwrap_or(f64::NAN)
}

/// `ToInt32` of an arbitrary [`Value`] (coerce via [`to_number_value`], then [`to_int32`]). Designed
/// to compose in generated code: unlike a `let Value::Number(a) = &(..) else { panic!() }` block,
/// it binds no name, so nested bitwise/shift operands can never shadow each other.
#[inline]
pub fn to_int32_value(v: &Value) -> i32 {
    to_int32(to_number_value(v))
}

/// `ToUint32` companion to [`to_int32_value`].
#[inline]
pub fn to_uint32_value(v: &Value) -> u32 {
    to_uint32(to_number_value(v))
}

/// Invoke a callable [`Value`]: [`Value::Function`], or an object exposing `__call` (e.g. `Symbol`).
pub fn value_call(callee: &Value, args: &[Value]) -> Value {
    match callee {
        Value::Function(f) => f.call(args),
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
            Value::String(s) => write!(f, "String({:?})", s.as_str()),
            Value::Bool(b) => write!(f, "Bool({})", b),
            Value::Null => write!(f, "Null"),
            Value::Array(arr) => write!(f, "Array({:?})", arr.borrow()),
            Value::NumberArray(arr) => write!(f, "NumberArray({:?})", arr.borrow()),
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

/// Format an `f64` exactly like JavaScript's `Number.prototype.toString` (radix 10) — the
/// algorithm behind `console.log(n)` and `String(n)`. Rust's default `{}` never uses
/// exponential form and so prints `6.022e23` as `602200000000000000000000`; JS switches to
/// exponential when the decimal point lands past digit 21 or before digit −6.
///
/// We take the shortest round-tripping digits from Rust's `{:e}` (a Ryū/Grisu-class shortest
/// formatter, matching V8's digit choice) and lay them out per the ECMAScript rule: plain
/// decimal when the point position `n` is in `(-6, 21]`, otherwise `d[.ddd]e±E` with `E = n-1`
/// (sign always shown, no leading zeros in the exponent). This is the ECMAScript `Number::toString`
/// used by `.toString()`, `String(n)`, `+` concatenation, and template literals, so `-0` renders as
/// `"0"` (the sign is dropped — `(-0).toString() === "0"`). The *inspect* form (`console.log` of a
/// bare number / array element) keeps `"-0"`; that distinction lives in [`Value::to_display_string`].
pub fn js_number_to_string(value: f64) -> String {
    let mut out = String::new();
    js_number_to_string_into(&mut out, value);
    out
}

/// Append the ECMAScript `Number::toString` of `value` to `out` — identical result to
/// [`js_number_to_string`] but with no intermediate `String` allocation. The JSON.stringify
/// hot path appends millions of numbers into a single buffer, so the alloc-free form matters.
pub fn js_number_to_string_into(out: &mut String, value: f64) {
    if value.is_nan() {
        out.push_str("NaN");
        return;
    }
    if value == f64::INFINITY {
        out.push_str("Infinity");
        return;
    }
    if value == f64::NEG_INFINITY {
        out.push_str("-Infinity");
        return;
    }
    if value == 0.0 {
        // ECMAScript `Number::toString`: both `+0` and `-0` stringify to `"0"`.
        out.push('0');
        return;
    }

    // Fast path for exact integers within ±2^53 (the f64 "safe integer" range). ECMAScript ToString
    // renders any such integer in plain decimal (no sign-exponent — magnitude < 1e21 ⇒ `point <= 21`
    // in the general algorithm below), which `itoa` emits directly, bypassing the `format!("{:e}")`
    // round-trip + intermediate `String`/`char`-filter/`"0".repeat()`. This is the dominant case for
    // tally/counter/index loops (e.g. map keys like `"w" + (n % 1000)`). Bit-identical to the general
    // path for every integer it accepts: each such value is exactly representable in both f64 and i64,
    // so `value as i64` is exact (no saturation — `±2^53` is well inside i64), and the decimal form is
    // the same digits the general path would emit. Larger integers (where f64 can't represent
    // consecutive values) fall through to the general path unchanged.
    const MAX_SAFE_INT: f64 = 9_007_199_254_740_992.0; // 2^53
    if value.fract() == 0.0 && value.abs() <= MAX_SAFE_INT {
        let mut buf = itoa::Buffer::new();
        out.push_str(buf.format(value as i64));
        return;
    }

    let negative = value < 0.0;
    // Shortest round-trip digits via ryu, then reassembled into ECMAScript `Number::toString` form.
    // ryu writes the shortest correctly-rounded digits straight into a stack buffer with none of the
    // `core::fmt` `{:e}` Formatter machinery the previous version paid per call (profiled as ~40% of
    // JSON.stringify self-time on numeric payloads). number→string is the JSON / template-literal /
    // logging / `+=` primitive, so this is a broad win. The shortest round-trip digit *sequence* is
    // unique, so ryu emits the same digits as the old `{:e}` path → output stays byte-identical.
    let mut ryu_buf = ryu::Buffer::new();
    // Finite & non-zero here (NaN / ±∞ / ±0 handled above), so `format_finite` is safe and cheaper.
    let s = ryu_buf.format_finite(value.abs());

    // Parse ryu's `<int>[.<frac>][e<exp>]` into significant digits + ECMAScript decimal point.
    // `point` = ECMAScript's `n`: value = digits × 10^(point − k), with `k` = significant-digit count.
    let (mant, exp) = match s.as_bytes().iter().position(|&c| c == b'e' || c == b'E') {
        Some(i) => (&s[..i], s[i + 1..].parse::<i32>().expect("ryu exponent")),
        None => (s, 0i32),
    };
    let (int_part, frac_part) = match mant.as_bytes().iter().position(|&c| c == b'.') {
        Some(i) => (&mant[..i], &mant[i + 1..]),
        None => (mant, ""),
    };
    let frac_len = frac_part.len() as i32;
    // Raw digit run = int_part ++ frac_part (ryu's mantissa, ≤ ~18 chars for an f64).
    let mut digits_buf = [0u8; 32];
    let mut n = 0usize;
    for &c in int_part.as_bytes().iter().chain(frac_part.as_bytes()) {
        digits_buf[n] = c;
        n += 1;
    }
    // Strip leading zeros (e.g. ryu "0.1" → raw "01" → "1") …
    let mut lead = 0usize;
    while lead + 1 < n && digits_buf[lead] == b'0' {
        lead += 1;
    }
    // … and trailing zeros (defensive — shortest form has none — folded back via `trail`).
    let mut end = n;
    while end - 1 > lead && digits_buf[end - 1] == b'0' {
        end -= 1;
    }
    let trail = (n - end) as i32;
    let digits = std::str::from_utf8(&digits_buf[lead..end]).expect("ascii digits");
    let k = digits.len() as i32; // significant digit count (≤ 17 for an f64)
    let point = k + exp - frac_len + trail;

    if negative {
        out.push('-');
    }
    if k <= point && point <= 21 {
        // Integer, zero-padded: digits then (point − k) trailing zeros.
        out.push_str(digits);
        for _ in 0..(point - k) {
            out.push('0');
        }
    } else if 0 < point && point <= 21 {
        // Decimal point inside the digit string.
        out.push_str(&digits[..point as usize]);
        out.push('.');
        out.push_str(&digits[point as usize..]);
    } else if -6 < point && point <= 0 {
        // Leading-zero fraction: "0." then (−point) zeros then the digits.
        out.push_str("0.");
        for _ in 0..(-point) {
            out.push('0');
        }
        out.push_str(digits);
    } else {
        // Exponential: first digit, optional `.rest`, then `e±E`.
        let e = point - 1;
        out.push_str(&digits[..1]);
        if k > 1 {
            out.push('.');
            out.push_str(&digits[1..]);
        }
        out.push('e');
        out.push(if e >= 0 { '+' } else { '-' });
        let mut ib = itoa::Buffer::new();
        out.push_str(ib.format(e.abs()));
    }
}

impl Value {
    /// Convert value to display string (for console output).
    pub fn to_display_string(&self) -> String {
        self.to_display_string_guarded(&mut Vec::new())
    }

    /// Cycle-safe recursive form (#381): `ancestors` holds the current path's container-cell pointers,
    /// so a self-referential array/object (`a.self = a`) renders `[Circular]` instead of recursing
    /// forever and wedging the thread. Ancestor-only (not all-visited), so a shared DAG node still
    /// renders in full.
    fn to_display_string_guarded(&self, ancestors: &mut Vec<*const ()>) -> String {
        match self {
            // Inspect form keeps the sign of negative zero (`console.log(-0)` → `-0`), unlike the
            // ECMAScript ToString used by `to_js_string`. See `js_number_to_string`. (#247)
            Value::Number(n) if *n == 0.0 && n.is_sign_negative() => "-0".to_string(),
            Value::Number(n) => js_number_to_string(*n),
            Value::String(s) => s.to_string(),
            Value::Bool(b) => b.to_string(),
            Value::Null => "null".to_string(),
            Value::Array(arr) => {
                let ptr = arr.as_ptr();
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
            Value::NumberArray(arr) => {
                let inner: Vec<String> = arr
                    .borrow()
                    .iter()
                    .map(|&n| if n.is_nan() { "null".to_string() } else { Value::Number(n).to_display_string() })
                    .collect();
                format!("[{}]", inner.join(", "))
            }
            Value::Object(obj) => {
                let ptr = obj.as_ptr();
                if ancestors.contains(&ptr) {
                    return "[Circular]".to_string();
                }
                ancestors.push(ptr);
                let inner: Vec<String> = obj
                    .borrow()
                    .strings
                    .iter()
                    .map(|(k, v)| format!("{}: {}", k.as_ref(), v.to_display_string_guarded(ancestors)))
                    .collect();
                ancestors.pop();
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

    /// JavaScript `ToString` coercion (the value's `.toString()`), as used by `Array.prototype.join`,
    /// string concatenation, and template literals — distinct from [`Self::to_display_string`], which
    /// is the *inspect/console* form (arrays bracketed, strings quoted in some contexts). The key
    /// JS-conformance differences from display: a nested **array** stringifies to its own
    /// comma-joined `toString` (recursively, always `,` regardless of the outer separator), an
    /// **object** becomes `"[object Object]"`, and primitives render as their plain value. `null`
    /// renders as `"null"` here (matching `String(null)`); `join` itself maps `null`/`undefined`
    /// elements to `""` *before* calling this, per the spec.
    pub fn to_js_string(&self) -> String {
        self.to_js_string_guarded(&mut Vec::new())
    }

    /// Cycle-safe `ToString` (#381): only arrays recurse here (objects render `[object Object]`), so a
    /// cyclic array via `"" + a` would otherwise hang. A back-reference joins as `""` — matching V8's
    /// `Array.prototype.join`, where a self-referential element contributes the empty string.
    fn to_js_string_guarded(&self, ancestors: &mut Vec<*const ()>) -> String {
        match self {
            Value::Array(arr) => {
                let ptr = arr.as_ptr();
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
            Value::NumberArray(arr) => arr
                .borrow()
                .iter()
                .map(|n| Value::Number(*n).to_js_string())
                .collect::<Vec<_>>()
                .join(","),
            Value::Object(_) => "[object Object]".to_string(),
            // ECMAScript ToString of a number (drops `-0`'s sign), distinct from the inspect form
            // that `to_display_string` would give for `-0`. (#247)
            Value::Number(n) => js_number_to_string(*n),
            // Primitives (and the remaining cases) coincide with the display form.
            _ => self.to_display_string(),
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
            (Value::NumberArray(a), Value::NumberArray(b)) => VmRef::ptr_eq(a, b),
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
        Value::Function(Arc::new(FnCallable(f)))
    }

    #[cfg(not(feature = "send-values"))]
    pub fn native<F>(f: F) -> Self
    where
        F: Fn(&[Value]) -> Value + 'static,
    {
        Value::Function(std::rc::Rc::new(FnCallable(f)))
    }

    /// Create a new array Value from a Vec.
    pub fn array(items: Vec<Value>) -> Self {
        Value::Array(VmRef::new(items))
    }

    /// Create a new object Value from a property map.
    pub fn object(map: ObjectMap) -> Self {
        Value::Object(VmRef::new(ObjectData::from_strings(map)))
    }

    /// Create an object directly from key/value pairs, building the `PropMap`
    /// in one pass with **no intermediate `AHashMap`**. Used by the Rust
    /// backend's object-literal codegen and any hot path that knows its pairs,
    /// so small objects (the common case) cost a single inline allocation.
    pub fn object_from_pairs<const N: usize>(pairs: [(Arc<str>, Value); N]) -> Self {
        let mut strings = PropMap::with_capacity(N);
        for (k, v) in pairs {
            strings.insert(k, v);
        }
        Value::Object(VmRef::new(ObjectData {
            strings,
            symbols: None,
        }))
    }

    /// Create an empty array Value.
    pub fn empty_array() -> Self {
        Value::Array(VmRef::new(Vec::new()))
    }

    // -------------------------------------------------------------------------
    // Packed f64 array support (TISH_PACKED_ARRAYS)
    // -------------------------------------------------------------------------

    /// Whether packed f64 arrays are enabled this run. Default: **off** (`TISH_PACKED_ARRAYS=1`
    /// opts in). Read once per process and cached — the VM calls this once per executed array
    /// literal, and a `std::env::var` there is a libc env lock plus a `String` allocation per
    /// `NewArray` for a flag that never changes after startup (#166). Set the variable before the
    /// process starts (as the CI sweep does); mid-process toggling is not observed by design.
    /// The flag is intentionally backwards from the slot/JIT flags (those were default-on) to
    /// keep the default binary behaviour byte-identical while we validate coverage.
    #[inline]
    pub fn packed_arrays_enabled() -> bool {
        static PACKED_ARRAYS: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *PACKED_ARRAYS.get_or_init(|| {
            std::env::var("TISH_PACKED_ARRAYS").map(|v| v == "1").unwrap_or(false)
        })
    }

    /// Wrap a `Vec<f64>` as a `Value::NumberArray`. Only call when `packed_arrays_enabled()`.
    #[inline]
    pub fn number_array(items: Vec<f64>) -> Self {
        Value::NumberArray(VmRef::new(items))
    }

    /// Materialize a `Value::NumberArray` into a boxed `Value::Array`.
    /// Called on the deopt path: any operation that doesn't have a packed fast path
    /// (non-numeric push, getIndex-beyond-bounds, spread into non-numeric context, etc.)
    /// converts once and continues on the generic path. The original `NumberArray` VmRef
    /// is consumed; callers replace the `Value` in whatever container held it.
    #[inline]
    pub fn materialize_number_array(arr: &VmRef<Vec<f64>>) -> Value {
        let nums = arr.borrow();
        Value::Array(VmRef::new(nums.iter().map(|&n| Value::Number(n)).collect()))
    }

    /// If `self` is a `NumberArray`, materialise and return `Value::Array`; otherwise
    /// return `self` unchanged. Convenience deopt for callers that pattern-match on `Array`.
    #[inline]
    pub fn coerce_number_array(self) -> Value {
        match self {
            Value::NumberArray(ref arr) => Value::materialize_number_array(arr),
            other => other,
        }
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
            Value::Array(_) | Value::NumberArray(_) => "object",
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

#[cfg(test)]
mod value_abstraction_tests {
    // #201 Stage A — the abstraction must exactly mirror the enum it will replace.
    use super::{Value, ValueRef, ValueTag};

    #[test]
    fn named_ctors_report_the_right_tag() {
        assert_eq!(Value::number(1.5).tag(), ValueTag::Number);
        assert_eq!(Value::boolean(true).tag(), ValueTag::Bool);
        assert_eq!(Value::string("hi").tag(), ValueTag::String);
        assert_eq!(Value::null().tag(), ValueTag::Null);
        assert_eq!(Value::array(vec![]).tag(), ValueTag::Array);
        assert_eq!(Value::empty_object().tag(), ValueTag::Object);
    }

    #[test]
    fn unpack_roundtrips_each_variant() {
        assert!(matches!(Value::number(3.0).unpack(), ValueRef::Number(n) if n == 3.0));
        assert!(matches!(Value::boolean(false).unpack(), ValueRef::Bool(false)));
        assert!(matches!(Value::string("x").unpack(), ValueRef::String(s) if s.as_str() == "x"));
        assert!(matches!(Value::null().unpack(), ValueRef::Null));
        assert!(matches!(Value::array(vec![]).unpack(), ValueRef::Array(_)));
        assert!(matches!(Value::empty_object().unpack(), ValueRef::Object(_)));
    }

    #[test]
    fn accessors_extract_or_none() {
        assert_eq!(Value::number(2.0).as_number(), Some(2.0));
        assert_eq!(Value::boolean(true).as_bool(), Some(true));
        assert_eq!(Value::string("abc").as_str(), Some("abc"));
        assert!(Value::null().is_null());
        // wrong-variant accessors return None
        assert_eq!(Value::number(1.0).as_bool(), None);
        assert_eq!(Value::null().as_str(), None);
        assert!(!Value::number(0.0).is_null());
        assert!(Value::array(vec![Value::number(1.0)]).as_array().is_some());
        assert!(Value::number(1.0).as_array().is_none());
    }
}

#[cfg(test)]
mod number_to_string_tests {
    use super::js_number_to_string;

    #[test]
    fn matches_javascript_number_tostring() {
        // (value, expected) — every `expected` is what Node's `String(value)` produces.
        // `String(-0) === "0"`: ToString drops the sign of negative zero (the inspect form keeps
        // it, but that path is `Value::to_display_string`, not this function). (#247)
        let cases: &[(f64, &str)] = &[
            (0.0, "0"),
            (-0.0, "0"),
            (123.0, "123"),
            (123.456, "123.456"),
            (0.5, "0.5"),
            (-123.456, "-123.456"),
            (100000.0, "100000"),
            (-1000.0, "-1000"),
            // Integer fast-path boundary (±2^53): the largest safe integer takes the itoa path; the
            // next decade up (still an exact f64 integer, but > 2^53) takes the general path. Both must
            // match Node's `String(value)`.
            (9007199254740992.0, "9007199254740992"), // 2^53
            (9007199254740991.0, "9007199254740991"), // 2^53 - 1 (Number.MAX_SAFE_INTEGER)
            (-9007199254740992.0, "-9007199254740992"),
            (1e16, "10000000000000000"), // > 2^53, exact f64 integer → general path
            // Decimal/exponential boundary on the large side: 1e21 flips to exponential.
            (1e20, "100000000000000000000"),
            (1e21, "1e+21"),
            (21e18, "21000000000000000000"),
            // Small side: 1e-6 is decimal, 1e-7 is exponential.
            (1e-6, "0.000001"),
            (1e-7, "1e-7"),
            (9.5e-7, "9.5e-7"),
            // Exponential with a multi-digit mantissa.
            (6.022e23, "6.022e+23"),
            (1.2345678901234568e21, "1.2345678901234568e+21"),
            (1e100, "1e+100"),
            (-1e21, "-1e+21"),
            // Subnormal min and normal max.
            (5e-324, "5e-324"),
            (1.7976931348623157e308, "1.7976931348623157e+308"),
            // Shortest round-trip mantissa (not full precision).
            (0.1, "0.1"),
            (0.1 + 0.2, "0.30000000000000004"),
            // Shortest-representation ties → ECMAScript rounds the final digit to EVEN (spec: "if
            // there are two such possible values, choose the one that is even"). These large-magnitude
            // fractionals each have two equally-short round-tripping decimals; Node/V8 emit the even
            // one. Verified against Node's `String(value)`. (The prior `{:e}`-based path rounded the
            // other way — ryu does ties-to-even, matching the spec.)
            (2181495296738027.3, "2181495296738027.2"),
            (75251554695404.13, "75251554695404.12"),
            (256006004960902.63, "256006004960902.62"),
            (-18546578340962.313, "-18546578340962.312"),
            // Non-finite.
            (f64::INFINITY, "Infinity"),
            (f64::NEG_INFINITY, "-Infinity"),
            (f64::NAN, "NaN"),
        ];
        for &(value, expected) in cases {
            assert_eq!(js_number_to_string(value), expected, "for {value:?}");
        }
    }
}

#[cfg(test)]
mod cycle_coercion_tests_381 {
    //! #381: string-coercing a cyclic array/object must terminate (was a silent thread hang),
    //! matching Node — `console.log`/inspect renders `[Circular]`, `"" + a` renders the back-ref as
    //! the empty string via `Array.prototype.join`.
    use super::Value;
    use crate::VmRef;

    #[test]
    fn display_string_cyclic_array_terminates() {
        let a = Value::Array(VmRef::new(Vec::new()));
        if let Value::Array(inner) = &a {
            inner.borrow_mut().push(a.clone()); // a = [a]
        }
        assert_eq!(a.to_display_string(), "[[Circular]]");
    }

    #[test]
    fn js_string_cyclic_array_terminates() {
        let a = Value::Array(VmRef::new(Vec::new()));
        if let Value::Array(inner) = &a {
            inner.borrow_mut().push(a.clone());
        }
        // Node: `let a=[]; a.push(a); "" + a` === "" (the self-referential element joins as "").
        assert_eq!(a.to_js_string(), "");
    }

    #[test]
    fn display_string_cyclic_object_terminates() {
        let o = Value::object_from_pairs([]);
        if let Value::Object(inner) = &o {
            inner.borrow_mut().strings.insert(std::sync::Arc::from("self"), o.clone());
        }
        assert_eq!(o.to_display_string(), "{self: [Circular]}");
    }

    #[test]
    fn display_string_shared_dag_is_not_circular() {
        let shared = Value::Array(VmRef::new(vec![Value::Number(1.0)]));
        let root = Value::Array(VmRef::new(vec![shared.clone(), shared.clone()]));
        assert_eq!(root.to_display_string(), "[[1], [1]]", "a shared node is a DAG, not a cycle");
    }
}

#[cfg(test)]
mod propmap_merge_tests {
    use super::{PropMap, Value, PROPMAP_INLINE};
    use std::sync::Arc;

    fn pm(pairs: &[(&str, f64)]) -> PropMap {
        let mut m = PropMap::default();
        for (k, v) in pairs {
            m.insert(Arc::from(*k), Value::Number(*v));
        }
        m
    }

    fn keys(m: &PropMap) -> Vec<String> {
        m.keys().map(|k| k.to_string()).collect()
    }

    fn num(m: &PropMap, k: &str) -> Option<f64> {
        match m.get(k) {
            Some(Value::Number(n)) => Some(*n),
            _ => None,
        }
    }

    #[test]
    fn merge_appends_new_keys_in_source_order() {
        let mut dst = pm(&[("a", 1.0), ("b", 2.0)]);
        dst.merge_from(&pm(&[("c", 3.0), ("d", 4.0)]));
        assert_eq!(keys(&dst), ["a", "b", "c", "d"]);
        assert_eq!(num(&dst, "c"), Some(3.0));
    }

    #[test]
    fn merge_overwrites_existing_keys_without_reordering() {
        // `{ ...{a,b,c}, b: 20 }` — later write wins, key keeps its original slot.
        let mut dst = pm(&[("a", 1.0), ("b", 2.0), ("c", 3.0)]);
        dst.merge_from(&pm(&[("b", 20.0), ("d", 4.0)]));
        assert_eq!(keys(&dst), ["a", "b", "c", "d"]);
        assert_eq!(num(&dst, "b"), Some(20.0));
        assert_eq!(num(&dst, "d"), Some(4.0));
    }

    #[test]
    fn merge_empty_source_is_noop() {
        let mut dst = pm(&[("a", 1.0)]);
        dst.merge_from(&PropMap::default());
        assert_eq!(keys(&dst), ["a"]);
    }

    #[test]
    fn merge_promotes_past_inline_threshold_preserving_order_and_overrides() {
        // Combined unique key count exceeds the inline cap, forcing the IndexMap promotion path.
        let mut dst = PropMap::default();
        for i in 0..PROPMAP_INLINE {
            dst.insert(Arc::from(format!("k{i}").as_str()), Value::Number(i as f64));
        }
        let mut src = PropMap::default();
        // Overwrite one existing key and add several new ones to cross the threshold.
        src.insert(Arc::from("k0"), Value::Number(100.0));
        for i in PROPMAP_INLINE..(PROPMAP_INLINE + 5) {
            src.insert(Arc::from(format!("k{i}").as_str()), Value::Number(i as f64));
        }
        dst.merge_from(&src);
        assert_eq!(dst.len(), PROPMAP_INLINE + 5);
        assert_eq!(num(&dst, "k0"), Some(100.0)); // override applied
        assert_eq!(num(&dst, "k0").is_some(), keys(&dst).first().map(|k| k == "k0").unwrap());
        // Original keys come first (insertion order), then the new ones.
        let ks = keys(&dst);
        assert_eq!(ks[0], "k0");
        assert_eq!(ks[PROPMAP_INLINE], format!("k{PROPMAP_INLINE}"));
        // Lookups remain correct after promotion.
        for i in PROPMAP_INLINE..(PROPMAP_INLINE + 5) {
            assert_eq!(num(&dst, &format!("k{i}")), Some(i as f64));
        }
    }

    #[test]
    fn merge_into_empty_matches_clone() {
        let src = pm(&[("x", 7.0), ("y", 8.0)]);
        let mut dst = PropMap::default();
        dst.merge_from(&src);
        assert_eq!(keys(&dst), keys(&src));
        assert_eq!(num(&dst, "x"), Some(7.0));
        assert_eq!(num(&dst, "y"), Some(8.0));
    }
}

