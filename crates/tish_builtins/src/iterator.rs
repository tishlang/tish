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
