//! `tish:test` and `tish:assert` module export maps.

use tishlang_core::{value_call, Arc, ObjectMap, Value};

use crate::assert::assert_object;
use crate::expect::expect_object;
use crate::mocks::{clear_all_mocks, mock, restore_all_mocks, spy_on};
use crate::registry::{add_hook, add_test, push_describe, TestMode};

fn name_and_fn(args: &[Value]) -> (String, Value) {
    match args.len() {
        0 => (String::new(), Value::Null),
        1 => {
            if matches!(args[0], Value::Function(_) | Value::Object(_)) {
                ("<anonymous>".into(), args[0].clone())
            } else {
                (args[0].to_display_string(), Value::Null)
            }
        }
        _ => {
            let name = args[0].to_display_string();
            let body = args.last().cloned().unwrap_or(Value::Null);
            (name, body)
        }
    }
}

fn opts_object(args: &[Value]) -> Option<Value> {
    if args.len() >= 3 {
        if let Value::Object(_) = &args[1] {
            return Some(args[1].clone());
        }
    }
    None
}

fn timeout_from_args(args: &[Value]) -> Option<u64> {
    let v = opts_object(args)?;
    let Value::Object(o) = v else {
        return None;
    };
    let n = {
        let b = o.borrow();
        b.strings
            .get("timeout")
            .and_then(|v| v.as_number())
            .filter(|n| n.is_finite() && *n >= 0.0)
    }?;
    Some(n as u64)
}

fn tags_from_args(args: &[Value]) -> Vec<String> {
    let Some(Value::Object(o)) = opts_object(args) else {
        return Vec::new();
    };
    let b = o.borrow();
    match b.strings.get("tags") {
        Some(Value::Array(a)) => a.borrow().iter().map(|v| v.to_display_string()).collect(),
        Some(Value::String(s)) => vec![s.to_string()],
        _ => Vec::new(),
    }
}

fn is_truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => *n != 0.0 && !n.is_nan(),
        Value::String(s) => !s.is_empty(),
        _ => true,
    }
}

fn row_args(row: &Value) -> Vec<Value> {
    match row {
        Value::Array(a) => a.borrow().clone(),
        Value::NumberArray(a) => a.borrow().to_values(),
        other => vec![other.clone()],
    }
}

fn format_each_name(template: &str, row: &Value, index: usize) -> String {
    if !template.contains('%') {
        return format!("{template} [{index}]");
    }
    let args = row_args(row);
    let mut out = String::new();
    let mut chars = template.chars().peekable();
    let mut ai = 0usize;
    while let Some(c) = chars.next() {
        if c == '%' {
            match chars.peek().copied() {
                Some('#') => {
                    chars.next();
                    out.push_str(&index.to_string());
                }
                Some('s') | Some('i') | Some('d') | Some('p') | Some('j') => {
                    chars.next();
                    let v = args.get(ai).cloned().unwrap_or(Value::Null);
                    ai += 1;
                    out.push_str(&v.to_display_string());
                }
                Some('%') => {
                    chars.next();
                    out.push('%');
                }
                _ => out.push('%'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn make_test_call(mode: TestMode) -> Value {
    Value::native(move |args: &[Value]| {
        let (name, body) = name_and_fn(args);
        let timeout = timeout_from_args(args);
        let tags = tags_from_args(args);
        // Parallel `test.concurrent` is deferred (single-threaded VM); `{ concurrent: true }`
        // is accepted and ignored so Bun/Jest suites port without edits.
        add_test(&name, mode, body, timeout, tags);
        Value::Null
    })
}

fn make_each_call(mode: TestMode) -> Value {
    Value::native(move |args: &[Value]| {
        let table = args.first().cloned().unwrap_or(Value::Null);
        Value::native(move |inner: &[Value]| {
            let (name_tmpl, body) = name_and_fn(inner);
            let timeout = timeout_from_args(inner);
            let rows: Vec<Value> = match &table {
                Value::Array(a) => a.borrow().clone(),
                Value::NumberArray(a) => a.borrow().to_values(),
                other => vec![other.clone()],
            };
            for (i, row) in rows.into_iter().enumerate() {
                let name = format_each_name(&name_tmpl, &row, i);
                let body = body.clone();
                let row_for_call = row;
                let wrapped = Value::native(move |_a: &[Value]| {
                    let call_args = row_args(&row_for_call);
                    value_call(&body, &call_args)
                });
                add_test(&name, mode, wrapped, timeout, Vec::new());
            }
            Value::Null
        })
    })
}

fn attach_each(target: &mut ObjectMap, mode: TestMode) {
    target.insert(Arc::from("each"), make_each_call(mode));
}

fn test_object() -> Value {
    let mut m = ObjectMap::default();
    m.insert(Arc::from("__call"), make_test_call(TestMode::Normal));
    attach_each(&mut m, TestMode::Normal);
    m.insert(Arc::from("skipIf"), {
        Value::native(|args: &[Value]| {
            let skip = args.first().map(is_truthy).unwrap_or(false);
            make_test_call(if skip {
                TestMode::Skip
            } else {
                TestMode::Normal
            })
        })
    });
    m.insert(Arc::from("skip"), {
        let mut o = ObjectMap::default();
        o.insert(Arc::from("__call"), make_test_call(TestMode::Skip));
        attach_each(&mut o, TestMode::Skip);
        Value::object(o)
    });
    m.insert(Arc::from("only"), {
        let mut o = ObjectMap::default();
        o.insert(Arc::from("__call"), make_test_call(TestMode::Only));
        attach_each(&mut o, TestMode::Only);
        Value::object(o)
    });
    m.insert(Arc::from("todo"), {
        let mut o = ObjectMap::default();
        o.insert(
            Arc::from("__call"),
            Value::native(|args: &[Value]| {
                let name = args
                    .first()
                    .map(|v| v.to_display_string())
                    .unwrap_or_else(|| "<todo>".into());
                add_test(&name, TestMode::Todo, Value::Null, None, Vec::new());
                Value::Null
            }),
        );
        Value::object(o)
    });
    m.insert(Arc::from("failing"), {
        let mut o = ObjectMap::default();
        o.insert(Arc::from("__call"), make_test_call(TestMode::Failing));
        attach_each(&mut o, TestMode::Failing);
        Value::object(o)
    });
    Value::object(m)
}

fn make_describe_call(mode: TestMode) -> Value {
    Value::native(move |args: &[Value]| {
        let name = args
            .first()
            .map(|v| v.to_display_string())
            .unwrap_or_default();
        let body = args.get(1).cloned().unwrap_or(Value::Null);
        push_describe(&name, mode, &body);
        Value::Null
    })
}

fn describe_object() -> Value {
    let mut m = ObjectMap::default();
    m.insert(Arc::from("__call"), make_describe_call(TestMode::Normal));
    m.insert(Arc::from("skip"), {
        let mut o = ObjectMap::default();
        o.insert(Arc::from("__call"), make_describe_call(TestMode::Skip));
        Value::object(o)
    });
    m.insert(Arc::from("only"), {
        let mut o = ObjectMap::default();
        o.insert(Arc::from("__call"), make_describe_call(TestMode::Only));
        Value::object(o)
    });
    // `describe.each` — table-driven suites (same table shape as `test.each`).
    m.insert(
        Arc::from("each"),
        Value::native(move |args: &[Value]| {
            let table = args.first().cloned().unwrap_or(Value::Null);
            Value::native(move |inner: &[Value]| {
                let name_tmpl = inner
                    .first()
                    .map(|v| v.to_display_string())
                    .unwrap_or_default();
                let body = inner.get(1).cloned().unwrap_or(Value::Null);
                let rows: Vec<Value> = match &table {
                    Value::Array(a) => a.borrow().clone(),
                    Value::NumberArray(a) => a.borrow().to_values(),
                    other => vec![other.clone()],
                };
                for (i, row) in rows.into_iter().enumerate() {
                    let name = format_each_name(&name_tmpl, &row, i);
                    let body = body.clone();
                    let row_for_call = row;
                    let wrapped = Value::native(move |_a: &[Value]| {
                        let call_args = row_args(&row_for_call);
                        value_call(&body, &call_args)
                    });
                    push_describe(&name, TestMode::Normal, &wrapped);
                }
                Value::Null
            })
        }),
    );
    Value::object(m)
}

fn hook(kind: &'static str) -> Value {
    Value::native(move |args: &[Value]| {
        let body = args.first().cloned().unwrap_or(Value::Null);
        add_hook(kind, body);
        Value::Null
    })
}

/// Named exports for `import { … } from "tish:test"`.
pub fn test_module() -> ObjectMap {
    let test = test_object();
    let describe = describe_object();
    let expect = expect_object();

    let mut m = ObjectMap::default();
    m.insert(Arc::from("describe"), describe.clone());
    m.insert(Arc::from("suite"), describe);
    m.insert(Arc::from("test"), test.clone());
    m.insert(Arc::from("it"), test);
    m.insert(Arc::from("expect"), expect);
    m.insert(Arc::from("beforeAll"), hook("beforeAll"));
    m.insert(Arc::from("afterAll"), hook("afterAll"));
    m.insert(Arc::from("beforeEach"), hook("beforeEach"));
    m.insert(Arc::from("afterEach"), hook("afterEach"));
    m.insert(Arc::from("before"), hook("before"));
    m.insert(Arc::from("after"), hook("after"));
    m.insert(Arc::from("mock"), Value::native(mock));
    m.insert(Arc::from("spyOn"), Value::native(spy_on));
    m.insert(Arc::from("clearAllMocks"), Value::native(clear_all_mocks));
    m.insert(
        Arc::from("restoreAllMocks"),
        Value::native(restore_all_mocks),
    );
    let default_test = m.get("test").cloned().unwrap_or(Value::Null);
    m.insert(Arc::from("default"), default_test);
    m
}

/// Named + default exports for `tish:assert` / `node:assert`.
pub fn assert_module() -> ObjectMap {
    let assert = assert_object();
    let mut m = ObjectMap::default();
    if let Value::Object(o) = &assert {
        for (k, v) in o.borrow().strings.iter() {
            if k.as_ref() != "__call" {
                m.insert(Arc::clone(k), v.clone());
            }
        }
    }
    m.insert(Arc::from("assert"), assert.clone());
    m.insert(Arc::from("default"), assert);
    m
}
