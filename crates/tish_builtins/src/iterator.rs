//! JS-style iterator objects for `Map`/`Set` (`.values()` / `.keys()` / `.entries()`).
//!
//! Like [`crate::collections`] and [`crate::date`], an iterator is a plain `Value::Object`
//! whose `next` method is a per-instance `Value::native` closure capturing a snapshot of the
//! items plus a shared position cell — so the one implementation works on every backend with
//! no new `Value` variant. `next()` returns `{ value, done }`; once exhausted it keeps
//! returning `{ value: null, done: true }`.
//!
//! The runtimes drive iteration through [`tishlang_core::drain_iterator`], which calls `next()`
//! until `done` — that's how these objects work in `for…of`, spread, and `Array.from`. The
//! snapshot is taken when the iterator is built (`.values()` is called), matching how the old
//! array-returning version behaved; mid-iteration mutation of the source is not reflected (a
//! live iterator is a follow-up).

use std::sync::Arc;

use tishlang_core::{ObjectMap, Value, VmRef};

/// Build a single-use iterator object over `items`: `{ next() -> { value, done } }`.
pub fn array_iterator(items: Vec<Value>) -> Value {
    let items: VmRef<Vec<Value>> = VmRef::new(items);
    let pos: VmRef<usize> = VmRef::new(0);

    let mut m = ObjectMap::default();
    {
        // Bulk-drain fast path for `for…of` / spread: return all REMAINING items (from the current
        // position) as one array and exhaust the iterator — equivalent to calling `next()` until
        // `done`, but with no per-element `{ value, done }` allocation. `drain_iterator` prefers this
        // when present; manual `.next()` and partial consumption still work (it respects `pos`).
        let items = items.clone();
        let pos = pos.clone();
        m.insert(
            Arc::from("__drain__"),
            Value::native(move |_args: &[Value]| {
                let i = *pos.borrow();
                let b = items.borrow();
                let rest: Vec<Value> = if i < b.len() { b[i..].to_vec() } else { Vec::new() };
                let len = b.len();
                drop(b);
                *pos.borrow_mut() = len;
                Value::Array(VmRef::new(rest))
            }),
        );
    }
    {
        let items = items.clone();
        let pos = pos.clone();
        m.insert(
            Arc::from("next"),
            Value::native(move |_args: &[Value]| {
                let i = *pos.borrow();
                // Read the element (or note exhaustion) without holding the items borrow
                // across the position write — two different `VmRef`s, but keep it tidy.
                let (value, done) = {
                    let b = items.borrow();
                    if i < b.len() {
                        (b[i].clone(), false)
                    } else {
                        (Value::Null, true)
                    }
                };
                if !done {
                    *pos.borrow_mut() = i + 1;
                }
                let mut r = ObjectMap::default();
                r.insert(Arc::from("value"), value);
                r.insert(Arc::from("done"), Value::Bool(done));
                Value::object(r)
            }),
        );
    }
    Value::object(m)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(obj: &Value, name: &str) -> Value {
        let Value::Object(o) = obj else { panic!("not an object") };
        let m = o.borrow().strings.get(name).cloned().expect("method missing");
        let Value::Function(f) = m else { panic!("{name} is not callable") };
        f.call(&[])
    }
    fn get(obj: &Value, key: &str) -> Value {
        let Value::Object(o) = obj else { return Value::Null };
        o.borrow().strings.get(key).cloned().unwrap_or(Value::Null)
    }
    fn num(v: &Value) -> f64 {
        match v {
            Value::Number(n) => *n,
            _ => f64::NAN,
        }
    }
    fn nums(v: &Value) -> Vec<f64> {
        match v {
            Value::Array(a) => a.borrow().iter().map(num).collect(),
            _ => vec![],
        }
    }

    #[test]
    fn next_yields_each_then_done() {
        let it = array_iterator(vec![Value::Number(1.0), Value::Number(2.0)]);
        let r1 = call(&it, "next");
        assert_eq!(num(&get(&r1, "value")), 1.0);
        assert!(!get(&r1, "done").is_truthy());
        assert_eq!(num(&get(&call(&it, "next"), "value")), 2.0);
        // exhausted — and it keeps reporting done.
        assert!(get(&call(&it, "next"), "done").is_truthy());
        assert!(get(&call(&it, "next"), "done").is_truthy());
    }

    #[test]
    fn drain_returns_all_and_exhausts() {
        let it = array_iterator(vec![Value::Number(1.0), Value::Number(2.0), Value::Number(3.0)]);
        assert_eq!(nums(&call(&it, "__drain__")), vec![1.0, 2.0, 3.0]);
        // draining exhausts the iterator (matches calling next() until done).
        assert!(get(&call(&it, "next"), "done").is_truthy());
    }

    #[test]
    fn drain_respects_current_position() {
        let it = array_iterator(vec![Value::Number(1.0), Value::Number(2.0), Value::Number(3.0)]);
        call(&it, "next"); // consume the first
        assert_eq!(nums(&call(&it, "__drain__")), vec![2.0, 3.0]);
        assert_eq!(nums(&call(&it, "__drain__")), Vec::<f64>::new()); // already drained
    }
}
