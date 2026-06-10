//! `Set` and `Map` — real constructors + methods for the non-JS targets (interpreter, VM, native).
//!
//! Like [`crate::date`], an instance is a plain `Value::Object` whose methods are per-instance
//! `Value::native` closures that capture a shared backing store, so the one implementation works
//! across every backend with no new `Value` variant. The backing store is an **insertion-ordered hash
//! map** ([`indexmap::IndexMap`]) keyed by [`Key`] (SameValueZero), so `get`/`set`/`has`/`add` are
//! O(1) average rather than the O(n) linear scan a `Vec`-of-pairs would force (a `Map` doing N
//! operations was previously O(N²) — see `docs/perf-benchmark-suite.md`). `delete` uses `shift_remove`
//! to preserve iteration order. Each instance also carries a hidden [`SIZE_SLOT`] opaque
//! ([`SizeProbe`]) wired to the live store so the runtimes can answer the computed `.size` property
//! via [`collection_size`].
//!
//! ## v1 scope / known gaps (documented in tishlang-web)
//! - `add`/`set` return `undefined` (no method chaining yet — native object methods receive no
//!   `this`).
//! - `.values()` / `.keys()` / `.entries()` return real **iterators** (objects with a `next()`
//!   that yields `{ value, done }`), so they work both directly (`it.next()`) and in `for…of` /
//!   spread / `Array.from` via [`tishlang_core::drain_iterator`]. The iterator snapshots the
//!   collection when created; mutating the source mid-iteration is not reflected (a live iterator
//!   is a follow-up). Direct `for (x of set)` (iterating the collection itself) and `forEach`
//!   (a callback the interpreter's closures cannot cross into a core native fn) are still
//!   follow-ups — iterate via `.values()` / `.entries()`.
//! - Keys/values use **SameValueZero** equality (NaN equals NaN; `+0`/`-0` are the same; objects by
//!   identity).

use std::any::Any;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use indexmap::IndexMap;
use tishlang_core::{NativeFn, ObjectMap, TishOpaque, Value, VmRef};

const CONSTRUCT: &str = "__construct";

/// Hidden instance slot holding a [`SizeProbe`] opaque so every runtime can answer `.size`. An
/// `Opaque` is *shared* (`Arc::clone`, not deep-copied) across the interpreter's core↔eval value
/// bridge, so the probe stays wired to the live backing store even after an instance is bridged —
/// a hidden `Value::Array` would be copied and go stale on the first mutation.
pub const SIZE_SLOT: &str = "__tish_size__";

/// SameValueZero-keyed entry wrapper, so a `Value` can key an insertion-ordered hash map with JS
/// `Map`/`Set` semantics: NaN equals NaN, `+0`/`-0` unify, primitives by value, references by
/// identity. `Hash` is kept consistent with this `Eq` (equal keys hash equal). Primitive keys
/// (number / string / bool / null — the common case) hash by value → true O(1). Reference keys hash
/// by a per-variant tag only and are disambiguated by `ptr_eq` in `Eq`; that keeps object-keyed maps
/// *correct* (if same-bucket within a variant) without needing a stable address out of `VmRef`.
#[derive(Clone)]
struct Key(Value);

impl PartialEq for Key {
    fn eq(&self, other: &Self) -> bool {
        same_value_zero(&self.0, &other.0)
    }
}
impl Eq for Key {}

impl Hash for Key {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match &self.0 {
            Value::Number(n) => {
                0u8.hash(state);
                // Canonicalize so `Eq` partners hash together: `+0` and `-0` share a key, and every
                // NaN is one key.
                let bits = if *n == 0.0 {
                    0u64
                } else if n.is_nan() {
                    0x7ff8_0000_0000_0000
                } else {
                    n.to_bits()
                };
                bits.hash(state);
            }
            Value::String(s) => {
                1u8.hash(state);
                s.as_str().hash(state);
            }
            Value::Bool(b) => {
                2u8.hash(state);
                b.hash(state);
            }
            Value::Null => 3u8.hash(state),
            // Reference / identity values: per-variant tag only; `ptr_eq` in `Eq` does the rest.
            Value::Symbol(_) => 4u8.hash(state),
            Value::Array(_) => 5u8.hash(state),
            Value::NumberArray(_) => 6u8.hash(state),
            Value::Object(_) => 7u8.hash(state),
            _ => 8u8.hash(state),
        }
    }
}

/// Shared backing for both `Set` and `Map`: an insertion-ordered hash map keyed by [`Key`]. A `Set`
/// stores `Value::Null` values and iterates its keys; a `Map` stores the mapped value. Uses `ahash`
/// (the same fast hasher `tish_core`'s `PropMap` uses) rather than the default SipHash — iteration
/// order is insertion order regardless of the hasher, so this is a pure constant-factor speedup.
type Store = VmRef<IndexMap<Key, Value, ahash::RandomState>>;

/// Opaque whose length reports a collection's live element count. Stored under [`SIZE_SLOT`] on every
/// `Set`/`Map` instance.
struct SizeProbe(Store);

impl TishOpaque for SizeProbe {
    fn type_name(&self) -> &'static str {
        "CollectionSize"
    }
    fn get_method(&self, _name: &str) -> Option<NativeFn> {
        None
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// If `op` is a [`SizeProbe`], its live length. Lets the interpreter (which holds the probe as an
/// already-bridged `Opaque`) read `.size` without depending on this module's private `SizeProbe`.
pub fn size_probe_len(op: &dyn TishOpaque) -> Option<f64> {
    op.as_any()
        .downcast_ref::<SizeProbe>()
        .map(|p| p.0.borrow().len() as f64)
}

/// Wrap a backing store as the hidden [`SIZE_SLOT`] opaque. `Value::Opaque`'s payload is always
/// `Arc<dyn TishOpaque>`, so on the single-threaded build (`Rc`-based `VmRef`) clippy's
/// `arc_with_non_send_sync` fires spuriously — the `Arc` is mandated by the API, not a thread choice.
#[allow(clippy::arc_with_non_send_sync)]
fn size_slot(store: &Store) -> Value {
    Value::Opaque(Arc::new(SizeProbe(store.clone())))
}

/// SameValueZero — the equality `Set`/`Map` use for membership. NaN equals NaN; `+0` and `-0` are
/// equal; primitives compare by value; reference types compare by identity.
fn same_value_zero(a: &Value, b: &Value) -> bool {
    use Value::*;
    match (a, b) {
        (Number(x), Number(y)) => x == y || (x.is_nan() && y.is_nan()),
        (String(x), String(y)) => x == y,
        (Bool(x), Bool(y)) => x == y,
        (Null, Null) => true,
        (Symbol(x), Symbol(y)) => Arc::ptr_eq(x, y),
        (Array(x), Array(y)) => VmRef::ptr_eq(x, y),
        (NumberArray(x), NumberArray(y)) => VmRef::ptr_eq(x, y),
        (Object(x), Object(y)) => VmRef::ptr_eq(x, y),
        _ => false,
    }
}

/// Iterate a `Value` as a sequence: arrays yield their elements; everything else yields nothing.
fn iter_elements(v: &Value) -> Vec<Value> {
    match v {
        Value::Array(a) => a.borrow().iter().cloned().collect(),
        Value::NumberArray(a) => a.borrow().iter().map(|n| Value::Number(*n)).collect(),
        _ => Vec::new(),
    }
}

/// The live `.size` of a `Set`/`Map` instance, or `None` for any other value. Runtimes call this
/// from `get_prop` so `set.size` / `map.size` read as a plain number property.
pub fn collection_size(obj: &Value) -> Option<f64> {
    if let Value::Object(o) = obj {
        if let Some(Value::Opaque(op)) = o.borrow().strings.get(SIZE_SLOT) {
            return size_probe_len(op.as_ref());
        }
    }
    None
}

// ─────────────────────────────────────────── Set ───────────────────────────────────────────────

/// Build a `Set` instance object over a fresh backing store seeded from `initial`.
pub fn set_instance(initial: &[Value]) -> Value {
    let store: Store = VmRef::new(IndexMap::default());
    {
        let mut b = store.borrow_mut();
        for v in initial {
            // `or_insert` dedups and keeps first-insertion order.
            b.entry(Key(v.clone())).or_insert(Value::Null);
        }
    }
    let mut m = ObjectMap::default();
    // Hidden slot that drives `.size` (shared across the interp value bridge — see SIZE_SLOT).
    m.insert(Arc::from(SIZE_SLOT), size_slot(&store));

    {
        let s = store.clone();
        m.insert(
            Arc::from("add"),
            Value::native(move |args: &[Value]| {
                let v = args.first().cloned().unwrap_or(Value::Null);
                s.borrow_mut().entry(Key(v)).or_insert(Value::Null);
                Value::Null
            }),
        );
    }
    {
        let s = store.clone();
        m.insert(
            Arc::from("has"),
            Value::native(move |args: &[Value]| {
                let v = args.first().cloned().unwrap_or(Value::Null);
                Value::Bool(s.borrow().contains_key(&Key(v)))
            }),
        );
    }
    {
        let s = store.clone();
        m.insert(
            Arc::from("delete"),
            Value::native(move |args: &[Value]| {
                let v = args.first().cloned().unwrap_or(Value::Null);
                // `shift_remove` preserves iteration order (vs `swap_remove`).
                Value::Bool(s.borrow_mut().shift_remove(&Key(v)).is_some())
            }),
        );
    }
    {
        let s = store.clone();
        m.insert(
            Arc::from("clear"),
            Value::native(move |_args: &[Value]| {
                s.borrow_mut().clear();
                Value::Null
            }),
        );
    }
    // values()/keys() are identical for a Set (the elements are the keys); entries() yields [v, v].
    {
        let s = store.clone();
        let values = move |_args: &[Value]| {
            let out: Vec<Value> = s.borrow().keys().map(|k| k.0.clone()).collect();
            crate::iterator::array_iterator(out)
        };
        m.insert(Arc::from("values"), Value::native(values.clone()));
        m.insert(Arc::from("keys"), Value::native(values));
    }
    {
        let s = store.clone();
        m.insert(
            Arc::from("entries"),
            Value::native(move |_args: &[Value]| {
                let pairs: Vec<Value> = s
                    .borrow()
                    .keys()
                    .map(|k| Value::Array(VmRef::new(vec![k.0.clone(), k.0.clone()])))
                    .collect();
                crate::iterator::array_iterator(pairs)
            }),
        );
    }

    Value::object(m)
}

/// The global `Set` constructor (`new Set()` / `new Set([1, 2, 2])`).
pub fn set_constructor_value() -> Value {
    let mut m = ObjectMap::default();
    m.insert(
        Arc::from(CONSTRUCT),
        Value::native(|args: &[Value]| {
            let initial = args.first().map(iter_elements).unwrap_or_default();
            set_instance(&initial)
        }),
    );
    Value::object(m)
}

// ─────────────────────────────────────────── Map ───────────────────────────────────────────────

/// Build a `Map` instance object over a fresh backing store seeded from `pairs` (each element an
/// iterable `[k, v]`).
pub fn map_instance(pairs: &[Value]) -> Value {
    let store: Store = VmRef::new(IndexMap::default());
    {
        let mut b = store.borrow_mut();
        for p in pairs {
            let kv = iter_elements(p);
            let k = kv.first().cloned().unwrap_or(Value::Null);
            let v = kv.get(1).cloned().unwrap_or(Value::Null);
            b.insert(Key(k), v);
        }
    }
    let mut m = ObjectMap::default();
    m.insert(Arc::from(SIZE_SLOT), size_slot(&store));

    {
        let s = store.clone();
        m.insert(
            Arc::from("set"),
            Value::native(move |args: &[Value]| {
                let key = args.first().cloned().unwrap_or(Value::Null);
                let val = args.get(1).cloned().unwrap_or(Value::Null);
                // `insert` updates an existing key in place (keeps its position) or appends a new one.
                s.borrow_mut().insert(Key(key), val);
                Value::Null
            }),
        );
    }
    {
        let s = store.clone();
        m.insert(
            Arc::from("get"),
            Value::native(move |args: &[Value]| {
                let key = args.first().cloned().unwrap_or(Value::Null);
                s.borrow().get(&Key(key)).cloned().unwrap_or(Value::Null)
            }),
        );
    }
    {
        let s = store.clone();
        m.insert(
            Arc::from("has"),
            Value::native(move |args: &[Value]| {
                let key = args.first().cloned().unwrap_or(Value::Null);
                Value::Bool(s.borrow().contains_key(&Key(key)))
            }),
        );
    }
    {
        let s = store.clone();
        m.insert(
            Arc::from("delete"),
            Value::native(move |args: &[Value]| {
                let key = args.first().cloned().unwrap_or(Value::Null);
                Value::Bool(s.borrow_mut().shift_remove(&Key(key)).is_some())
            }),
        );
    }
    {
        let s = store.clone();
        m.insert(
            Arc::from("clear"),
            Value::native(move |_args: &[Value]| {
                s.borrow_mut().clear();
                Value::Null
            }),
        );
    }
    {
        let s = store.clone();
        m.insert(
            Arc::from("keys"),
            Value::native(move |_args: &[Value]| {
                let out: Vec<Value> = s.borrow().keys().map(|k| k.0.clone()).collect();
                crate::iterator::array_iterator(out)
            }),
        );
    }
    {
        let s = store.clone();
        m.insert(
            Arc::from("values"),
            Value::native(move |_args: &[Value]| {
                let out: Vec<Value> = s.borrow().values().cloned().collect();
                crate::iterator::array_iterator(out)
            }),
        );
    }
    {
        let s = store.clone();
        m.insert(
            Arc::from("entries"),
            Value::native(move |_args: &[Value]| {
                let pairs: Vec<Value> = s
                    .borrow()
                    .iter()
                    .map(|(k, v)| Value::Array(VmRef::new(vec![k.0.clone(), v.clone()])))
                    .collect();
                crate::iterator::array_iterator(pairs)
            }),
        );
    }

    Value::object(m)
}

/// The global `Map` constructor (`new Map()` / `new Map([[k, v], …])`).
pub fn map_constructor_value() -> Value {
    let mut m = ObjectMap::default();
    m.insert(
        Arc::from(CONSTRUCT),
        Value::native(|args: &[Value]| {
            let pairs = args.first().map(iter_elements).unwrap_or_default();
            map_instance(&pairs)
        }),
    );
    Value::object(m)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn num(v: &Value) -> f64 {
        match v {
            Value::Number(n) => *n,
            _ => f64::NAN,
        }
    }

    #[test]
    fn set_dedups_and_counts() {
        let s = set_instance(&[Value::Number(1.0), Value::Number(2.0), Value::Number(2.0)]);
        assert_eq!(collection_size(&s), Some(2.0));
    }

    #[test]
    fn set_nan_is_one_member() {
        let s = set_instance(&[Value::Number(f64::NAN), Value::Number(f64::NAN)]);
        assert_eq!(collection_size(&s), Some(1.0));
    }

    #[test]
    fn map_size_via_hook() {
        let m = map_instance(&[
            Value::Array(VmRef::new(vec![Value::String("x".into()), Value::Number(1.0)])),
            Value::Array(VmRef::new(vec![Value::String("y".into()), Value::Number(2.0)])),
        ]);
        assert_eq!(collection_size(&m), Some(2.0));
    }

    #[test]
    fn key_same_value_zero_semantics() {
        // `+0` and `-0` are one key; NaN is one key.
        let mut map: IndexMap<Key, Value> = IndexMap::default();
        map.insert(Key(Value::Number(0.0)), Value::Number(1.0));
        map.insert(Key(Value::Number(-0.0)), Value::Number(2.0)); // updates the same entry
        assert_eq!(map.len(), 1);
        assert_eq!(num(map.get(&Key(Value::Number(0.0))).unwrap()), 2.0);

        map.insert(Key(Value::Number(f64::NAN)), Value::Number(7.0));
        map.insert(Key(Value::Number(f64::NAN)), Value::Number(8.0)); // same NaN key
        assert_eq!(num(map.get(&Key(Value::Number(f64::NAN))).unwrap()), 8.0);
        assert_eq!(map.len(), 2);
    }

    #[test]
    fn map_insert_updates_in_place_and_keeps_order() {
        let store: Store = VmRef::new(IndexMap::default());
        store
            .borrow_mut()
            .insert(Key(Value::String("a".into())), Value::Number(1.0));
        store
            .borrow_mut()
            .insert(Key(Value::String("a".into())), Value::Number(9.0)); // update, not insert
        store
            .borrow_mut()
            .insert(Key(Value::String("b".into())), Value::Number(2.0));
        let b = store.borrow();
        assert_eq!(b.len(), 2);
        assert_eq!(num(b.get(&Key(Value::String("a".into()))).unwrap()), 9.0);
        let order: Vec<&str> = b
            .keys()
            .map(|k| match &k.0 {
                Value::String(s) => s.as_str(),
                _ => "?",
            })
            .collect();
        assert_eq!(order, vec!["a", "b"]); // insertion order preserved
    }

    #[test]
    fn delete_preserves_order() {
        let store: Store = VmRef::new(IndexMap::default());
        for (k, v) in [("a", 1.0), ("b", 2.0), ("c", 3.0)] {
            store
                .borrow_mut()
                .insert(Key(Value::String(k.into())), Value::Number(v));
        }
        store
            .borrow_mut()
            .shift_remove(&Key(Value::String("b".into())));
        let b = store.borrow();
        let order: Vec<&str> = b
            .keys()
            .map(|k| match &k.0 {
                Value::String(s) => s.as_str(),
                _ => "?",
            })
            .collect();
        assert_eq!(order, vec!["a", "c"]); // "b" removed, order intact
    }
}
