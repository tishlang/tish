//! `Set` and `Map` — real constructors + methods for the non-JS targets (interpreter, VM, native).
//!
//! Like [`crate::date`], an instance is a plain `Value::Object` whose methods are per-instance
//! `Value::native` closures that capture a shared `VmRef<Vec<Value>>` backing store, so the one
//! implementation works across every backend with no new `Value` variant. Each instance also carries
//! a hidden [`SIZE_SLOT`] opaque ([`SizeProbe`]) wired to the live store so the runtimes can answer
//! the computed `.size` property via [`collection_size`].
//!
//! ## v1 scope / known gaps (documented in tishlang-web)
//! - `add`/`set` return `undefined` (no method chaining yet — native object methods receive no
//!   `this`).
//! - Iterate with `.values()` / `.keys()` / `.entries()` (each returns an array); direct
//!   `for (x of set)` / `[...set]` and `forEach` are not wired yet (`forEach` would pass a callback
//!   into a core native fn, which the interpreter's closures cannot cross — a callback bridge is a
//!   follow-up).
//! - Keys/values use **SameValueZero** equality (NaN equals NaN; `+0`/`-0` are the same; objects by
//!   identity).

use std::any::Any;
use std::sync::Arc;
use tishlang_core::{NativeFn, ObjectMap, TishOpaque, Value, VmRef};

const CONSTRUCT: &str = "__construct";

/// Hidden instance slot holding a [`SizeProbe`] opaque so every runtime can answer `.size`. An
/// `Opaque` is *shared* (`Arc::clone`, not deep-copied) across the interpreter's core↔eval value
/// bridge, so the probe stays wired to the live backing store even after an instance is bridged —
/// a hidden `Value::Array` would be copied and go stale on the first mutation.
pub const SIZE_SLOT: &str = "__tish_size__";

/// Opaque whose length reports a collection's live element count. Stored under [`SIZE_SLOT`] on
/// every `Set`/`Map` instance (a Map probes its keys store, which is kept in lockstep with values).
struct SizeProbe(VmRef<Vec<Value>>);

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
fn size_slot(store: &VmRef<Vec<Value>>) -> Value {
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
    let store: VmRef<Vec<Value>> = VmRef::new(Vec::new());
    {
        let mut b = store.borrow_mut();
        for v in initial {
            if !b.iter().any(|e| same_value_zero(e, v)) {
                b.push(v.clone());
            }
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
                let mut b = s.borrow_mut();
                if !b.iter().any(|e| same_value_zero(e, &v)) {
                    b.push(v);
                }
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
                Value::Bool(s.borrow().iter().any(|e| same_value_zero(e, &v)))
            }),
        );
    }
    {
        let s = store.clone();
        m.insert(
            Arc::from("delete"),
            Value::native(move |args: &[Value]| {
                let v = args.first().cloned().unwrap_or(Value::Null);
                let mut b = s.borrow_mut();
                if let Some(i) = b.iter().position(|e| same_value_zero(e, &v)) {
                    b.remove(i);
                    Value::Bool(true)
                } else {
                    Value::Bool(false)
                }
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
    // values()/keys() are identical for a Set; entries() yields [v, v] pairs.
    {
        let s = store.clone();
        let values = move |_args: &[Value]| Value::Array(VmRef::new(s.borrow().clone()));
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
                    .iter()
                    .map(|v| Value::Array(VmRef::new(vec![v.clone(), v.clone()])))
                    .collect();
                Value::Array(VmRef::new(pairs))
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

/// Build a `Map` instance object over fresh parallel key/value stores seeded from `pairs`
/// (each element an iterable `[k, v]`).
pub fn map_instance(pairs: &[Value]) -> Value {
    let keys: VmRef<Vec<Value>> = VmRef::new(Vec::new());
    let vals: VmRef<Vec<Value>> = VmRef::new(Vec::new());
    for p in pairs {
        let kv = iter_elements(p);
        let k = kv.first().cloned().unwrap_or(Value::Null);
        let v = kv.get(1).cloned().unwrap_or(Value::Null);
        map_set(&keys, &vals, k, v);
    }
    let mut m = ObjectMap::default();
    // `.size` probes the keys store (kept in lockstep with values).
    m.insert(Arc::from(SIZE_SLOT), size_slot(&keys));

    {
        let (k, v) = (keys.clone(), vals.clone());
        m.insert(
            Arc::from("set"),
            Value::native(move |args: &[Value]| {
                let key = args.first().cloned().unwrap_or(Value::Null);
                let val = args.get(1).cloned().unwrap_or(Value::Null);
                map_set(&k, &v, key, val);
                Value::Null
            }),
        );
    }
    {
        let (k, v) = (keys.clone(), vals.clone());
        m.insert(
            Arc::from("get"),
            Value::native(move |args: &[Value]| {
                let key = args.first().cloned().unwrap_or(Value::Null);
                match k.borrow().iter().position(|e| same_value_zero(e, &key)) {
                    Some(i) => v.borrow()[i].clone(),
                    None => Value::Null,
                }
            }),
        );
    }
    {
        let k = keys.clone();
        m.insert(
            Arc::from("has"),
            Value::native(move |args: &[Value]| {
                let key = args.first().cloned().unwrap_or(Value::Null);
                Value::Bool(k.borrow().iter().any(|e| same_value_zero(e, &key)))
            }),
        );
    }
    {
        let (k, v) = (keys.clone(), vals.clone());
        m.insert(
            Arc::from("delete"),
            Value::native(move |args: &[Value]| {
                let key = args.first().cloned().unwrap_or(Value::Null);
                // Bind the position FIRST so the read borrow is released before `borrow_mut`
                // (otherwise the live read guard deadlocks the mutable borrow).
                let pos = k.borrow().iter().position(|e| same_value_zero(e, &key));
                if let Some(i) = pos {
                    k.borrow_mut().remove(i);
                    v.borrow_mut().remove(i);
                    Value::Bool(true)
                } else {
                    Value::Bool(false)
                }
            }),
        );
    }
    {
        let (k, v) = (keys.clone(), vals.clone());
        m.insert(
            Arc::from("clear"),
            Value::native(move |_args: &[Value]| {
                k.borrow_mut().clear();
                v.borrow_mut().clear();
                Value::Null
            }),
        );
    }
    {
        let k = keys.clone();
        m.insert(
            Arc::from("keys"),
            Value::native(move |_args: &[Value]| Value::Array(VmRef::new(k.borrow().clone()))),
        );
    }
    {
        let v = vals.clone();
        m.insert(
            Arc::from("values"),
            Value::native(move |_args: &[Value]| Value::Array(VmRef::new(v.borrow().clone()))),
        );
    }
    {
        let (k, v) = (keys.clone(), vals.clone());
        m.insert(
            Arc::from("entries"),
            Value::native(move |_args: &[Value]| {
                let ks = k.borrow();
                let vs = v.borrow();
                let pairs: Vec<Value> = ks
                    .iter()
                    .zip(vs.iter())
                    .map(|(key, val)| Value::Array(VmRef::new(vec![key.clone(), val.clone()])))
                    .collect();
                Value::Array(VmRef::new(pairs))
            }),
        );
    }

    Value::object(m)
}

/// Insert-or-update `key`→`val` into the parallel key/value stores (SameValueZero key match).
fn map_set(keys: &VmRef<Vec<Value>>, vals: &VmRef<Vec<Value>>, key: Value, val: Value) {
    let pos = keys.borrow().iter().position(|e| same_value_zero(e, &key));
    match pos {
        Some(i) => vals.borrow_mut()[i] = val,
        None => {
            keys.borrow_mut().push(key);
            vals.borrow_mut().push(val);
        }
    }
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
    fn map_set_get_update() {
        let keys = VmRef::new(Vec::new());
        let vals = VmRef::new(Vec::new());
        map_set(&keys, &vals, Value::String("a".into()), Value::Number(1.0));
        map_set(&keys, &vals, Value::String("a".into()), Value::Number(9.0)); // update, not insert
        map_set(&keys, &vals, Value::String("b".into()), Value::Number(2.0));
        assert_eq!(keys.borrow().len(), 2);
        let i = keys
            .borrow()
            .iter()
            .position(|k| same_value_zero(k, &Value::String("a".into())))
            .unwrap();
        assert_eq!(num(&vals.borrow()[i]), 9.0);
    }

    #[test]
    fn map_size_via_hook() {
        let m = map_instance(&[
            Value::Array(VmRef::new(vec![Value::String("x".into()), Value::Number(1.0)])),
            Value::Array(VmRef::new(vec![Value::String("y".into()), Value::Number(2.0)])),
        ]);
        assert_eq!(collection_size(&m), Some(2.0));
    }
}
