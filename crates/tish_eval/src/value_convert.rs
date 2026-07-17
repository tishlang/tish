//! Conversion between tishlang_eval::Value and tishlang_core::Value for opaque method calls.

use std::cell::RefCell;
use std::rc::{Rc, Weak};
use std::sync::Arc;

#[cfg(feature = "regex")]
use tishlang_core::TishRegExp;
use ahash::{AHashMap, AHashSet};
use tishlang_core::{ObjectData, Value as CoreValue, VmRef};

use crate::value::{EvalObjectData, PropMap, Value};

/// #466 — identity-stable bridging for reference values (objects, arrays).
///
/// Core `Map`/`Set` key equality is SameValueZero (`VmRef::ptr_eq` for objects/arrays), so a fresh
/// `VmRef` per `eval_to_core` call made `m.get(k)`/`s.has(a)` fail for ANY object/array key in the
/// interpreter: the stored key and the lookup probe were different core allocations. This cache
/// gives every eval `Rc` a single stable core **twin**: repeat bridges return the same `VmRef`
/// (identity holds), with contents refreshed from the eval side on each bridge (preserving the
/// existing copy-at-the-boundary semantics — and matching vm/node more closely on
/// mutate-after-insert, since the stored key's view updates with its object).
///
/// The reverse maps let `core_to_eval` return the ORIGINAL eval object when a forward-bridged twin
/// flows back out (`m.get(k) === v`, `[...m.keys()][0] === k` — identity across the round trip,
/// as on vm/native/node).
///
/// Soundness against address reuse (the #250 lesson): forward entries pair the twin with a `Weak`
/// to the eval `Rc` and every hit verifies `upgrade() + Rc::ptr_eq`; a dead/differing weak means
/// the address was reused and the entry is replaced. Reverse hits re-verify through the forward
/// entry (`VmRef::ptr_eq`), and a live forward entry holds the twin strongly, so a live reverse
/// key's address cannot have been recycled. `in_flight` makes conversion cycle-safe: a
/// self/mutually-referential object hit mid-refresh returns its twin as-is instead of recursing
/// forever (the enclosing refresh completes the contents).
struct BridgeCache {
    obj_fwd: AHashMap<usize, (Weak<RefCell<EvalObjectData>>, VmRef<ObjectData>)>,
    arr_fwd: AHashMap<usize, (Weak<RefCell<Vec<Value>>>, VmRef<Vec<CoreValue>>)>,
    obj_rev: AHashMap<usize, Weak<RefCell<EvalObjectData>>>,
    arr_rev: AHashMap<usize, Weak<RefCell<Vec<Value>>>>,
    /// Eval addresses currently being refreshed (cycle guard).
    in_flight: AHashSet<usize>,
    /// Amortized cleanup threshold: sweep dead entries when the cache outgrows it.
    high_water: usize,
}

impl BridgeCache {
    fn new() -> Self {
        Self {
            obj_fwd: AHashMap::default(),
            arr_fwd: AHashMap::default(),
            obj_rev: AHashMap::default(),
            arr_rev: AHashMap::default(),
            in_flight: AHashSet::default(),
            high_water: 256,
        }
    }

    /// Drop entries whose eval side has died. Called on the insert path only; amortized O(1).
    fn maybe_sweep(&mut self) {
        if self.obj_fwd.len() + self.arr_fwd.len() <= self.high_water {
            return;
        }
        self.obj_fwd.retain(|_, (w, _)| w.upgrade().is_some());
        self.arr_fwd.retain(|_, (w, _)| w.upgrade().is_some());
        self.obj_rev.retain(|_, w| w.upgrade().is_some());
        self.arr_rev.retain(|_, w| w.upgrade().is_some());
        self.high_water = ((self.obj_fwd.len() + self.arr_fwd.len()) * 2).max(256);
    }
}

thread_local! {
    static BRIDGE: RefCell<BridgeCache> = RefCell::new(BridgeCache::new());
}

/// Convert interpreter Value to core Value. Fails for interpreter-only variants.
pub fn eval_to_core(v: &Value) -> Result<CoreValue, String> {
    match v {
        Value::Number(n) => Ok(CoreValue::Number(*n)),
        Value::String(s) => Ok(CoreValue::String(tishlang_core::ArcStr::from(s.as_ref()))),
        Value::Bool(b) => Ok(CoreValue::Bool(*b)),
        Value::Null => Ok(CoreValue::Null),
        Value::Array(arr) => {
            let addr = Rc::as_ptr(arr) as usize;
            // Cycle guard: this array is already being refreshed higher up this same
            // conversion — hand back its twin as-is; the enclosing refresh fills it.
            if let Some(twin) = BRIDGE.with(|c| {
                let c = c.borrow();
                if c.in_flight.contains(&addr) {
                    c.arr_fwd
                        .get(&addr)
                        .filter(|(w, _)| w.upgrade().is_some_and(|up| Rc::ptr_eq(&up, arr)))
                        .map(|(_, t)| t.clone())
                } else {
                    None
                }
            }) {
                return Ok(CoreValue::Array(twin));
            }
            // Resolve (or mint) the stable core twin for this eval Rc, then refresh contents.
            let twin = BRIDGE.with(|c| {
                let mut c = c.borrow_mut();
                match c.arr_fwd.get(&addr) {
                    Some((w, t)) if w.upgrade().is_some_and(|up| Rc::ptr_eq(&up, arr)) => {
                        t.clone()
                    }
                    _ => {
                        c.maybe_sweep();
                        let t: VmRef<Vec<CoreValue>> = VmRef::new(Vec::new());
                        c.arr_fwd.insert(addr, (Rc::downgrade(arr), t.clone()));
                        c.arr_rev.insert(t.as_ptr() as usize, Rc::downgrade(arr));
                        t
                    }
                }
            });
            BRIDGE.with(|c| c.borrow_mut().in_flight.insert(addr));
            let contents = (|| -> Result<Vec<CoreValue>, String> {
                let mut out = Vec::new();
                for item in arr.borrow().iter() {
                    out.push(eval_to_core(item)?);
                }
                Ok(out)
            })();
            BRIDGE.with(|c| {
                c.borrow_mut().in_flight.remove(&addr);
            });
            *twin.borrow_mut() = contents?;
            Ok(CoreValue::Array(twin))
        }
        Value::Object(map) => {
            let addr = Rc::as_ptr(map) as usize;
            // Cycle guard — see the Array arm.
            if let Some(twin) = BRIDGE.with(|c| {
                let c = c.borrow();
                if c.in_flight.contains(&addr) {
                    c.obj_fwd
                        .get(&addr)
                        .filter(|(w, _)| w.upgrade().is_some_and(|up| Rc::ptr_eq(&up, map)))
                        .map(|(_, t)| t.clone())
                } else {
                    None
                }
            }) {
                return Ok(CoreValue::Object(twin));
            }
            let twin = BRIDGE.with(|c| {
                let mut c = c.borrow_mut();
                match c.obj_fwd.get(&addr) {
                    Some((w, t)) if w.upgrade().is_some_and(|up| Rc::ptr_eq(&up, map)) => {
                        t.clone()
                    }
                    _ => {
                        c.maybe_sweep();
                        let t: VmRef<ObjectData> = VmRef::new(ObjectData {
                            strings: tishlang_core::PropMap::default(),
                            symbols: None,
                        });
                        c.obj_fwd.insert(addr, (Rc::downgrade(map), t.clone()));
                        c.obj_rev.insert(t.as_ptr() as usize, Rc::downgrade(map));
                        t
                    }
                }
            });
            BRIDGE.with(|c| c.borrow_mut().in_flight.insert(addr));
            let contents = (|| -> Result<ObjectData, String> {
                let b = map.borrow();
                let mut strings = tishlang_core::PropMap::default();
                for (k, v) in b.strings.iter() {
                    strings.insert(Arc::clone(k), eval_to_core(v)?);
                }
                let symbols = if let Some(ss) = &b.symbols {
                    let mut sm = AHashMap::default();
                    for (id, v) in ss.iter() {
                        sm.insert(*id, eval_to_core(v)?);
                    }
                    Some(sm)
                } else {
                    None
                };
                Ok(ObjectData { strings, symbols })
            })();
            BRIDGE.with(|c| {
                c.borrow_mut().in_flight.remove(&addr);
            });
            *twin.borrow_mut() = contents?;
            Ok(CoreValue::Object(twin))
        }
        Value::Symbol(s) => Ok(CoreValue::Symbol(Arc::clone(s))),
        Value::Opaque(o) => Ok(CoreValue::Opaque(Arc::clone(o))),
        _ => Err(format!(
            "Cannot pass {:?} to native function (unsupported type)",
            std::mem::discriminant(v)
        )),
    }
}

/// Convert core Value to interpreter Value.
pub fn core_to_eval(v: CoreValue) -> Value {
    match v {
        CoreValue::Number(n) => Value::Number(n),
        CoreValue::String(s) => Value::String(Arc::from(s.as_str())),
        CoreValue::Bool(b) => Value::Bool(b),
        CoreValue::Null => Value::Null,
        CoreValue::Array(arr) => {
            // #466: a forward-bridged twin flowing back out returns the ORIGINAL eval array, so
            // identity survives the round trip (`s.has(a)` after `[...s.keys()]`, `x === a`).
            if let Some(orig) = BRIDGE.with(|c| {
                let c = c.borrow();
                c.arr_rev
                    .get(&(arr.as_ptr() as usize))
                    .and_then(|w| w.upgrade())
                    .filter(|orig| {
                        c.arr_fwd
                            .get(&(Rc::as_ptr(orig) as usize))
                            .is_some_and(|(_, t)| VmRef::ptr_eq(t, &arr))
                    })
            }) {
                return Value::Array(orig);
            }
            let mut out = Vec::new();
            for item in arr.borrow().iter() {
                out.push(core_to_eval(item.clone()));
            }
            Value::Array(Rc::new(RefCell::new(out)))
        }
        CoreValue::Object(map) => {
            // #466: see the Array arm.
            if let Some(orig) = BRIDGE.with(|c| {
                let c = c.borrow();
                c.obj_rev
                    .get(&(map.as_ptr() as usize))
                    .and_then(|w| w.upgrade())
                    .filter(|orig| {
                        c.obj_fwd
                            .get(&(Rc::as_ptr(orig) as usize))
                            .is_some_and(|(_, t)| VmRef::ptr_eq(t, &map))
                    })
            }) {
                return Value::Object(orig);
            }
            let b = map.borrow();
            let mut out = PropMap::default();
            for (k, v) in b.strings.iter() {
                out.insert(Arc::clone(k), core_to_eval(v.clone()));
            }
            let mut eod = EvalObjectData::from_strings(out);
            if let Some(ss) = &b.symbols {
                let mut es = AHashMap::default();
                for (id, v) in ss.iter() {
                    es.insert(*id, core_to_eval(v.clone()));
                }
                eod.symbols = Some(es);
            }
            Value::Object(Rc::new(RefCell::new(eod)))
        }
        CoreValue::Symbol(s) => Value::Symbol(Arc::clone(&s)),
        CoreValue::Opaque(o) => Value::Opaque(o),
        #[cfg(feature = "http")]
        CoreValue::Promise(p) => Value::CorePromise(Arc::clone(&p)),
        #[cfg(not(feature = "http"))]
        CoreValue::Promise(_) => Value::Null,
        // NumberArray: materialize to boxed Array for the interpreter (it has no packed path).
        CoreValue::NumberArray(arr) => {
            // Materialize to a boxed Array for the interpreter (it has no packed path). `to_values`
            // handles a packed or a deopted (#199) backing.
            let out: Vec<Value> = arr
                .borrow()
                .to_values()
                .into_iter()
                .map(|v| match v {
                    tishlang_core::Value::Number(n) => Value::Number(n),
                    _ => core_to_eval(v),
                })
                .collect();
            Value::Array(Rc::new(RefCell::new(out)))
        }
        // `CoreNativeFn` is feature-gated (Rc vs Arc), so use Clone::clone
        // which works for either.
        CoreValue::Function(f) => Value::CoreFn(f.clone()),
        // tishlang_core gets regex from http or regex features; handle RegExp when it exists
        #[cfg(any(feature = "http", feature = "regex"))]
        CoreValue::RegExp(re) => {
            #[cfg(feature = "regex")]
            {
                // Core uses `VmRef<TishRegExp>` (potentially `Arc<Mutex>`),
                // interpreter uses `Rc<RefCell<TishRegExp>>`. Clone the
                // inner state across so the two storage shapes can coexist.
                let inner: TishRegExp = re.borrow().clone();
                Value::RegExp(Rc::new(RefCell::new(inner)))
            }
            #[cfg(not(feature = "regex"))]
            {
                let _ = re;
                Value::Null
            }
        }
    }
}

#[cfg(test)]
mod bridge_identity_tests {
    use super::*;

    fn eval_obj(pairs: &[(&str, Value)]) -> Value {
        let mut strings = PropMap::default();
        for (k, v) in pairs {
            strings.insert(Arc::from(*k), v.clone());
        }
        Value::Object(Rc::new(RefCell::new(EvalObjectData::from_strings(strings))))
    }

    /// #466: the same eval object must bridge to the SAME core allocation every time —
    /// that's what makes it usable as a Map/Set key (SameValueZero is ptr_eq for objects).
    #[test]
    fn same_eval_object_bridges_to_same_core_twin() {
        let k = eval_obj(&[("id", Value::Number(1.0))]);
        let (CoreValue::Object(t1), CoreValue::Object(t2)) =
            (eval_to_core(&k).unwrap(), eval_to_core(&k).unwrap())
        else {
            panic!("expected core objects");
        };
        assert!(VmRef::ptr_eq(&t1, &t2), "twin identity must be stable across bridges");

        // Distinct eval objects get distinct twins.
        let k2 = eval_obj(&[("id", Value::Number(1.0))]);
        let CoreValue::Object(t3) = eval_to_core(&k2).unwrap() else { panic!() };
        assert!(!VmRef::ptr_eq(&t1, &t3), "distinct objects must not share a twin");
    }

    /// Re-bridging refreshes the twin's contents (copy-at-the-boundary semantics kept).
    #[test]
    fn rebridge_refreshes_twin_contents() {
        let k = eval_obj(&[("id", Value::Number(1.0))]);
        let CoreValue::Object(t1) = eval_to_core(&k).unwrap() else { panic!() };
        assert!(matches!(t1.borrow().strings.get("id"), Some(CoreValue::Number(n)) if *n == 1.0));

        if let Value::Object(o) = &k {
            o.borrow_mut().strings.insert(Arc::from("id"), Value::Number(42.0));
        }
        let CoreValue::Object(t2) = eval_to_core(&k).unwrap() else { panic!() };
        assert!(VmRef::ptr_eq(&t1, &t2));
        assert!(matches!(t2.borrow().strings.get("id"), Some(CoreValue::Number(n)) if *n == 42.0));
    }

    /// A forward-bridged twin converts BACK to the original eval object (round-trip identity:
    /// `m.get(k) === v` / `[...m.keys()][0] === k`).
    #[test]
    fn core_to_eval_returns_original_for_bridged_twin() {
        let k = eval_obj(&[("id", Value::Number(1.0))]);
        let core = eval_to_core(&k).unwrap();
        let back = core_to_eval(core);
        let (Value::Object(orig), Value::Object(round)) = (&k, &back) else { panic!() };
        assert!(Rc::ptr_eq(orig, round), "round trip must return the original eval object");

        // An unrelated core object (never forward-bridged) still gets a fresh conversion.
        let alien = CoreValue::Object(VmRef::new(ObjectData {
            strings: tishlang_core::PropMap::default(),
            symbols: None,
        }));
        assert!(matches!(core_to_eval(alien), Value::Object(_)));
    }

    /// Arrays get the same treatment (Set members / Map values).
    #[test]
    fn array_identity_and_roundtrip() {
        let a = Value::Array(Rc::new(RefCell::new(vec![Value::Number(1.0), Value::Number(2.0)])));
        let (CoreValue::Array(t1), CoreValue::Array(t2)) =
            (eval_to_core(&a).unwrap(), eval_to_core(&a).unwrap())
        else {
            panic!();
        };
        assert!(VmRef::ptr_eq(&t1, &t2));
        let back = core_to_eval(CoreValue::Array(t1));
        let (Value::Array(orig), Value::Array(round)) = (&a, &back) else { panic!() };
        assert!(Rc::ptr_eq(orig, round));
    }

    /// A self-referential object must terminate (insert-before-recurse) and its twin's
    /// self-property must be the twin itself — mirroring the eval-side cycle.
    #[test]
    fn cyclic_object_converts_without_recursing_forever() {
        let k = eval_obj(&[]);
        if let Value::Object(o) = &k {
            let self_ref = k.clone();
            o.borrow_mut().strings.insert(Arc::from("me"), self_ref);
        }
        let CoreValue::Object(twin) = eval_to_core(&k).unwrap() else { panic!() };
        let inner = twin.borrow().strings.get("me").cloned();
        let Some(CoreValue::Object(inner)) = inner else { panic!("self prop missing") };
        assert!(VmRef::ptr_eq(&twin, &inner), "cycle must map to the twin itself");
    }

    /// Dropping the eval object invalidates its cache entry: a NEW object (even if the allocator
    /// reuses the address) must get a fresh twin, never the stale one (the #250 lesson).
    #[test]
    fn dead_eval_object_never_resurrects_stale_twin() {
        let mut stale_twin = None;
        for _ in 0..64 {
            let k = eval_obj(&[("x", Value::Number(7.0))]);
            let CoreValue::Object(t) = eval_to_core(&k).unwrap() else { panic!() };
            if let Some(prev) = &stale_twin {
                assert!(
                    !VmRef::ptr_eq(prev, &t),
                    "a fresh eval object must never receive a dead object's twin"
                );
            }
            stale_twin = Some(t);
            // k drops here — its address is free for reuse by the next iteration.
        }
    }
}
