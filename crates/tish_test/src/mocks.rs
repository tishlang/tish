//! `mock` / `spyOn` with call tracking.

use tishlang_core::{value_call, Arc, ObjectMap, Value, VmRef};

use std::cell::RefCell;
use std::sync::atomic::{AtomicUsize, Ordering};

thread_local! {
    static MOCKS: RefCell<Vec<MockHandle>> = const { RefCell::new(Vec::new()) };
}

struct MockHandle {
    meta: Value,
    restore: Option<(Value, String, Value)>,
}

static MOCK_SEQ: AtomicUsize = AtomicUsize::new(1);

fn next_mock_id() -> usize {
    MOCK_SEQ.fetch_add(1, Ordering::SeqCst)
}

fn empty_calls_array() -> Value {
    Value::Array(VmRef::new(Vec::new()))
}

fn bump_calls(meta: &Value, args: &[Value]) {
    let Value::Object(o) = meta else {
        return;
    };
    let mut b = o.borrow_mut();
    if !b.strings.contains_key("calls") {
        b.strings.insert(Arc::from("calls"), empty_calls_array());
    }
    if let Some(Value::Array(arr)) = b.strings.get("calls") {
        arr.borrow_mut()
            .push(Value::Array(VmRef::new(args.to_vec())));
    }
    let count = match b.strings.get("calls") {
        Some(Value::Array(a)) => a.borrow().len() as f64,
        _ => 0.0,
    };
    b.strings
        .insert(Arc::from("callCount"), Value::Number(count));
}

fn make_mock_meta(impl_fn: Value) -> Value {
    let id = next_mock_id();
    let mut m = ObjectMap::default();
    m.insert(Arc::from("mockId"), Value::Number(id as f64));
    m.insert(Arc::from("calls"), empty_calls_array());
    m.insert(Arc::from("callCount"), Value::Number(0.0));
    m.insert(Arc::from("implementation"), impl_fn.clone());
    let meta = Value::object(m);
    if let Value::Object(o) = &meta {
        let o_clear = o.clone();
        let o_reset = o.clone();
        let impl_reset = impl_fn;
        o.borrow_mut().strings.insert(
            Arc::from("mockClear"),
            Value::native(move |_args: &[Value]| {
                let mut b = o_clear.borrow_mut();
                b.strings.insert(Arc::from("calls"), empty_calls_array());
                b.strings.insert(Arc::from("callCount"), Value::Number(0.0));
                Value::Null
            }),
        );
        o.borrow_mut().strings.insert(
            Arc::from("mockReset"),
            Value::native(move |_args: &[Value]| {
                let mut b = o_reset.borrow_mut();
                b.strings.insert(Arc::from("calls"), empty_calls_array());
                b.strings.insert(Arc::from("callCount"), Value::Number(0.0));
                b.strings
                    .insert(Arc::from("implementation"), impl_reset.clone());
                Value::Null
            }),
        );
    }
    meta
}

fn wrap_mock_fn(meta: Value) -> Value {
    let meta_call = meta.clone();
    Value::native(move |args: &[Value]| {
        bump_calls(&meta_call, args);
        let impl_fn = match &meta_call {
            Value::Object(o) => o
                .borrow()
                .strings
                .get("implementation")
                .cloned()
                .unwrap_or(Value::Null),
            _ => Value::Null,
        };
        value_call(&impl_fn, args)
    })
}

/// Set `implementation` on a mock's metadata object.
fn set_implementation(meta: &Value, f: Value) {
    if let Value::Object(o) = meta {
        o.borrow_mut()
            .strings
            .insert(Arc::from("implementation"), f);
    }
}

/// Jest/Bun put `mockClear` / `mockImplementation` / `mockReturnValue` on the mock function
/// itself, not on `.mock`. Attach both so either spelling works.
fn attach_mock_controls(outer: &mut ObjectMap, meta: &Value) {
    if let Value::Object(o) = meta {
        for name in ["mockClear", "mockReset"] {
            let existing = o.borrow().strings.get(name).cloned();
            if let Some(f) = existing {
                outer.insert(Arc::from(name), f);
            }
        }
    }
    let m_impl = meta.clone();
    outer.insert(
        Arc::from("mockImplementation"),
        Value::native(move |args: &[Value]| {
            set_implementation(&m_impl, args.first().cloned().unwrap_or(Value::Null));
            Value::Null
        }),
    );
    let m_once = meta.clone();
    outer.insert(
        Arc::from("mockImplementationOnce"),
        Value::native(move |args: &[Value]| {
            set_implementation(&m_once, args.first().cloned().unwrap_or(Value::Null));
            Value::Null
        }),
    );
    let m_ret = meta.clone();
    outer.insert(
        Arc::from("mockReturnValue"),
        Value::native(move |args: &[Value]| {
            let v = args.first().cloned().unwrap_or(Value::Null);
            set_implementation(&m_ret, Value::native(move |_a: &[Value]| v.clone()));
            Value::Null
        }),
    );
    let m_res = meta.clone();
    outer.insert(
        Arc::from("mockResolvedValue"),
        Value::native(move |args: &[Value]| {
            let v = args.first().cloned().unwrap_or(Value::Null);
            // No promise construction here: a resolved value is what `await` yields anyway.
            set_implementation(&m_res, Value::native(move |_a: &[Value]| v.clone()));
            Value::Null
        }),
    );
}

/// `mock(fn?)` — callable mock object with `.mock.calls` / `.mockClear`.
pub fn mock(args: &[Value]) -> Value {
    let impl_fn = args
        .first()
        .cloned()
        .unwrap_or_else(|| Value::native(|_a: &[Value]| Value::Null));
    let meta = make_mock_meta(impl_fn);
    let call = wrap_mock_fn(meta.clone());
    let mut outer = ObjectMap::default();
    outer.insert(Arc::from("__call"), call);
    outer.insert(Arc::from("mock"), meta.clone());
    attach_mock_controls(&mut outer, &meta);
    let obj = Value::object(outer);
    MOCKS.with(|m| {
        m.borrow_mut().push(MockHandle {
            meta,
            restore: None,
        });
    });
    obj
}

/// `spyOn(obj, methodName)` — wraps a method; restore via `restoreAllMocks`.
pub fn spy_on(args: &[Value]) -> Value {
    let target = args.first().cloned().unwrap_or(Value::Null);
    let prop = args
        .get(1)
        .map(|v| v.to_display_string())
        .unwrap_or_default();
    let original = match &target {
        Value::Object(o) => o
            .borrow()
            .strings
            .get(prop.as_str())
            .cloned()
            .unwrap_or(Value::Null),
        _ => Value::Null,
    };
    let meta = make_mock_meta(original.clone());
    let call = wrap_mock_fn(meta.clone());
    let mut outer = ObjectMap::default();
    outer.insert(Arc::from("__call"), call);
    outer.insert(Arc::from("mock"), meta.clone());
    attach_mock_controls(&mut outer, &meta);
    let spy = Value::object(outer);
    if !matches!(target, Value::Object(_)) {
        // Silently returning an unattached spy made the whole test a no-op.
        tishlang_core::set_pending_throw(crate::assert::assertion_error(
            format!(
                "spyOn(target, {prop:?}): target is not an object ({})",
                target.to_display_string()
            ),
            Some(target.clone()),
            None,
            "spyOn",
            true,
        ));
        return Value::Null;
    }
    if let Value::Object(o) = &target {
        o.borrow_mut()
            .strings
            .insert(Arc::from(prop.as_str()), spy.clone());
    }
    MOCKS.with(|m| {
        m.borrow_mut().push(MockHandle {
            meta,
            restore: Some((target, prop, original)),
        });
    });
    spy
}

pub fn clear_all_mocks(_args: &[Value]) -> Value {
    MOCKS.with(|m| {
        for h in m.borrow().iter() {
            if let Value::Object(o) = &h.meta {
                let mut b = o.borrow_mut();
                b.strings.insert(Arc::from("calls"), empty_calls_array());
                b.strings.insert(Arc::from("callCount"), Value::Number(0.0));
            }
        }
    });
    Value::Null
}

pub fn restore_all_mocks(_args: &[Value]) -> Value {
    MOCKS.with(|m| {
        let mut list = m.borrow_mut();
        // Restore newest-first: spying twice on one method must land back on the ORIGINAL,
        // not on the first spy.
        for h in list.drain(..).rev() {
            if let Some((Value::Object(o), prop, original)) = h.restore {
                o.borrow_mut()
                    .strings
                    .insert(Arc::from(prop.as_str()), original);
            }
        }
    });
    Value::Null
}

/// Drop all mock/spy handles between test files (restore spies first).
pub fn reset_mock_registry() {
    let _ = restore_all_mocks(&[]);
}

pub fn mock_native(args: &[Value]) -> Value {
    mock(args)
}

pub fn spy_on_native(args: &[Value]) -> Value {
    spy_on(args)
}
