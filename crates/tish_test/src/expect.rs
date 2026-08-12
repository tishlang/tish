//! Jest/Bun-shaped `expect` matchers.

use tishlang_core::{
    has_pending_throw, set_pending_throw, take_pending_throw, value_call, Arc, ObjectMap, Value,
};

use crate::assert::assertion_error;
use crate::deep_equal::deep_strict_equal;

fn fail(msg: String, actual: Value, expected: Option<Value>, op: &str) -> Value {
    set_pending_throw(assertion_error(msg, Some(actual), expected, op, true));
    Value::Null
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

/// Can `value_call` actually invoke this? Objects are callable only via a `__call` member.
pub(crate) fn is_callable(v: &Value) -> bool {
    match v {
        Value::Function(_) => true,
        Value::Object(o) => o.borrow().strings.contains_key("__call"),
        _ => false,
    }
}

/// Human-readable text for a thrown value: `Error`-shaped objects report their `message`.
fn error_text(err: &Value) -> String {
    if let Value::Object(o) = err {
        if let Some(Value::String(m)) = o.borrow().strings.get("message") {
            return m.to_string();
        }
    }
    err.to_display_string()
}

/// Does a thrown value satisfy `expected` (string substring, regex, `Error`-shaped object,
/// or a deep-equal value)? Node/Jest semantics — the previous version accepted *any* throw.
pub(crate) fn error_matches(err: &Value, expected: &Value) -> bool {
    match expected {
        Value::String(pat) => error_text(err).contains(pat.as_str()),
        #[cfg(feature = "regex")]
        Value::RegExp(re) => re.borrow_mut().test(&error_text(err)),
        // `{ message, name, … }` — every listed field must match.
        Value::Object(o) if !o.borrow().strings.contains_key(ASYMMETRIC_KEY) => {
            let want = o.borrow();
            if want.strings.is_empty() {
                return deep_strict_equal(err, expected);
            }
            let Value::Object(got) = err else {
                return false;
            };
            let got = got.borrow();
            want.strings.iter().all(|(k, wv)| match got.strings.get(k) {
                Some(gv) => deep_strict_equal(gv, wv),
                None => false,
            })
        }
        _ => deep_strict_equal(err, expected),
    }
}

fn describe_error_matcher(expected: &Value) -> String {
    match expected {
        Value::String(s) => format!("a message containing {:?}", s.as_str()),
        _ => expected.to_display_string(),
    }
}

fn type_of_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Symbol(_) => "symbol",
        Value::Function(_) => "function",
        _ => "object",
    }
}

/// Resolve a `toHaveProperty` path (`"a.b.0"` or `["a", "b", 0]`) to its value.
fn property_at(root: &Value, path: &Value) -> Option<Value> {
    let segments: Vec<String> = match path {
        Value::Array(a) => a.borrow().iter().map(|v| v.to_display_string()).collect(),
        other => other
            .to_display_string()
            .split('.')
            .map(|s| s.to_string())
            .collect(),
    };
    let mut cur = root.clone();
    for seg in segments {
        cur = match &cur {
            Value::Object(o) => o.borrow().strings.get(seg.as_str())?.clone(),
            Value::Array(a) => {
                let idx: usize = seg.parse().ok()?;
                a.borrow().get(idx)?.clone()
            }
            Value::NumberArray(a) => {
                let idx: usize = seg.parse().ok()?;
                a.borrow().to_values().get(idx)?.clone()
            }
            _ => return None,
        };
    }
    Some(cur)
}

fn length_of(v: &Value) -> Option<f64> {
    match v {
        Value::Array(a) => Some(a.borrow().len() as f64),
        Value::NumberArray(a) => Some(a.borrow().len() as f64),
        Value::String(s) => Some(s.chars().count() as f64),
        Value::Object(o) => o.borrow().strings.get("length").and_then(|x| x.as_number()),
        _ => None,
    }
}

/// Resolve Jest/Bun mock metadata from `expect(fn)` / `expect(fn.mock)`.
fn mock_meta(v: &Value) -> Option<Value> {
    let Value::Object(o) = v else {
        return None;
    };
    let b = o.borrow();
    if b.strings.contains_key("calls") && b.strings.contains_key("callCount") {
        return Some(v.clone());
    }
    b.strings.get("mock").cloned().and_then(|m| {
        if let Value::Object(mo) = &m {
            let mb = mo.borrow();
            if mb.strings.contains_key("calls") {
                return Some(m.clone());
            }
        }
        None
    })
}

fn mock_call_count(meta: &Value) -> f64 {
    let Value::Object(o) = meta else {
        return 0.0;
    };
    o.borrow()
        .strings
        .get("callCount")
        .and_then(|v| v.as_number())
        .unwrap_or(0.0)
}

fn mock_calls(meta: &Value) -> Vec<Value> {
    let Value::Object(o) = meta else {
        return Vec::new();
    };
    match o.borrow().strings.get("calls") {
        Some(Value::Array(a)) => a.borrow().clone(),
        _ => Vec::new(),
    }
}

// `!(a > b)` and friends are deliberate: they are TRUE when either side is NaN, which is what
// these matchers must report. `partial_cmp` would silently pass an incomparable pair.
#[allow(clippy::neg_cmp_op_on_partial_ord)]
fn matcher_object(actual: Value) -> Value {
    let actual_be = actual.clone();
    let actual_eq = actual.clone();
    let actual_strict = actual.clone();
    let actual_throw = actual.clone();
    let actual_match = actual.clone();
    let actual_contain = actual.clone();
    let actual_null = actual.clone();
    let actual_truthy = actual.clone();
    let actual_falsy = actual.clone();
    let actual_defined = actual.clone();
    let actual_len = actual.clone();
    let actual_gt = actual.clone();
    let actual_gte = actual.clone();
    let actual_lt = actual.clone();
    let actual_lte = actual.clone();
    let actual_snap = actual.clone();
    let actual_called = actual.clone();
    let actual_called_times = actual.clone();
    let actual_called_with = actual.clone();
    let actual_last_with = actual.clone();

    let mut m = ObjectMap::default();
    m.insert(
        Arc::from("toBe"),
        Value::native(move |args: &[Value]| {
            let expected = args.first().cloned().unwrap_or(Value::Null);
            if !actual_be.strict_eq(&expected) {
                return fail(
                    format!(
                        "expect(received).toBe(expected)\n\nExpected: {}\nReceived: {}",
                        expected.to_display_string(),
                        actual_be.to_display_string()
                    ),
                    actual_be.clone(),
                    Some(expected),
                    "toBe",
                );
            }
            Value::Null
        }),
    );
    m.insert(
        Arc::from("toEqual"),
        Value::native(move |args: &[Value]| {
            let expected = args.first().cloned().unwrap_or(Value::Null);
            if !deep_strict_equal(&actual_eq, &expected) {
                return fail(
                    format!(
                        "expect(received).toEqual(expected)\n\nExpected: {}\nReceived: {}",
                        expected.to_display_string(),
                        actual_eq.to_display_string()
                    ),
                    actual_eq.clone(),
                    Some(expected),
                    "toEqual",
                );
            }
            Value::Null
        }),
    );
    m.insert(
        Arc::from("toStrictEqual"),
        Value::native(move |args: &[Value]| {
            let expected = args.first().cloned().unwrap_or(Value::Null);
            if !deep_strict_equal(&actual_strict, &expected) {
                return fail(
                    format!(
                        "expect(received).toStrictEqual(expected)\n\nExpected: {}\nReceived: {}",
                        expected.to_display_string(),
                        actual_strict.to_display_string()
                    ),
                    actual_strict.clone(),
                    Some(expected),
                    "toStrictEqual",
                );
            }
            Value::Null
        }),
    );
    m.insert(
        Arc::from("toThrow"),
        Value::native(move |args: &[Value]| {
            // A non-callable would make `value_call` set a pending throw, which used to read
            // as "the code threw" and pass. Reject it up front instead.
            if !is_callable(&actual_throw) {
                return fail(
                    format!(
                        "expect(received).toThrow()\n\nReceived value is not a function: {}",
                        actual_throw.to_display_string()
                    ),
                    actual_throw.clone(),
                    None,
                    "toThrow",
                );
            }
            let _ = take_pending_throw();
            let _ = value_call(&actual_throw, &[]);
            if !has_pending_throw() {
                return fail(
                    "expect(received).toThrow()\n\nReceived function did not throw".into(),
                    actual_throw.clone(),
                    None,
                    "toThrow",
                );
            }
            let err = take_pending_throw().unwrap_or(Value::Null);
            if let Some(expected) = args.first() {
                if !error_matches(&err, expected) {
                    return fail(
                        format!(
                            "expect(received).toThrow(expected)\n\nExpected: {}\nReceived: {}",
                            describe_error_matcher(expected),
                            error_text(&err)
                        ),
                        err,
                        Some(expected.clone()),
                        "toThrow",
                    );
                }
            }
            Value::Null
        }),
    );
    m.insert(
        Arc::from("toMatch"),
        Value::native(move |args: &[Value]| {
            let s = actual_match.to_display_string();
            let pattern = args.first().cloned().unwrap_or(Value::Null);
            let ok = match &pattern {
                #[cfg(feature = "regex")]
                Value::RegExp(r) => r.borrow_mut().test(&s),
                Value::String(p) => s.contains(p.as_str()),
                other => s.contains(&other.to_display_string()),
            };
            if !ok {
                return fail(
                    format!(
                        "expect(received).toMatch(expected)\n\nExpected pattern: {}\nReceived: {}",
                        pattern.to_display_string(),
                        s
                    ),
                    actual_match.clone(),
                    Some(pattern),
                    "toMatch",
                );
            }
            Value::Null
        }),
    );
    m.insert(
        Arc::from("toContain"),
        Value::native(move |args: &[Value]| {
            let item = args.first().cloned().unwrap_or(Value::Null);
            let ok = match &actual_contain {
                Value::String(s) => s.as_str().contains(&item.to_display_string()),
                Value::Array(a) => a.borrow().iter().any(|v| v.strict_eq(&item)),
                Value::NumberArray(a) => a
                    .borrow()
                    .to_values()
                    .iter()
                    .any(|v| v.strict_eq(&item)),
                _ => false,
            };
            if !ok {
                return fail(
                    format!(
                        "expect(received).toContain(expected)\n\nExpected to contain: {}\nReceived: {}",
                        item.to_display_string(),
                        actual_contain.to_display_string()
                    ),
                    actual_contain.clone(),
                    Some(item),
                    "toContain",
                );
            }
            Value::Null
        }),
    );
    m.insert(
        Arc::from("toBeNull"),
        Value::native(move |_args: &[Value]| {
            if !matches!(actual_null, Value::Null) {
                return fail(
                    format!(
                        "expect(received).toBeNull()\n\nReceived: {}",
                        actual_null.to_display_string()
                    ),
                    actual_null.clone(),
                    Some(Value::Null),
                    "toBeNull",
                );
            }
            Value::Null
        }),
    );
    m.insert(
        Arc::from("toBeTruthy"),
        Value::native(move |_args: &[Value]| {
            if !is_truthy(&actual_truthy) {
                return fail(
                    format!(
                        "expect(received).toBeTruthy()\n\nReceived: {}",
                        actual_truthy.to_display_string()
                    ),
                    actual_truthy.clone(),
                    None,
                    "toBeTruthy",
                );
            }
            Value::Null
        }),
    );
    m.insert(
        Arc::from("toBeFalsy"),
        Value::native(move |_args: &[Value]| {
            if is_truthy(&actual_falsy) {
                return fail(
                    format!(
                        "expect(received).toBeFalsy()\n\nReceived: {}",
                        actual_falsy.to_display_string()
                    ),
                    actual_falsy.clone(),
                    None,
                    "toBeFalsy",
                );
            }
            Value::Null
        }),
    );
    // Tish has no `undefined`; `null` is the absence value, so these mirror `toBeNull`.
    // Always-passing / always-failing versions were traps: ported Node suites got no signal.
    m.insert(
        Arc::from("toBeDefined"),
        Value::native(move |_args: &[Value]| {
            if matches!(actual_defined, Value::Null) {
                return fail(
                    "expect(received).toBeDefined()\n\nReceived: null".into(),
                    actual_defined.clone(),
                    None,
                    "toBeDefined",
                );
            }
            Value::Null
        }),
    );
    let actual_undefined = actual.clone();
    m.insert(
        Arc::from("toBeUndefined"),
        Value::native(move |_args: &[Value]| {
            if !matches!(actual_undefined, Value::Null) {
                return fail(
                    format!(
                        "expect(received).toBeUndefined()\n\nTish has no undefined; null is the absence value.\nReceived: {}",
                        actual_undefined.to_display_string()
                    ),
                    actual_undefined.clone(),
                    None,
                    "toBeUndefined",
                );
            }
            Value::Null
        }),
    );
    let actual_close = actual.clone();
    m.insert(
        Arc::from("toBeCloseTo"),
        Value::native(move |args: &[Value]| {
            let expected = args.first().and_then(|v| v.as_number()).unwrap_or(0.0);
            let digits = args.get(1).and_then(|v| v.as_number()).unwrap_or(2.0);
            let act = actual_close.as_number().unwrap_or(f64::NAN);
            // Jest/Bun: |a - b| < 10^-digits / 2. The un-halved form accepted values Jest rejects.
            let prec = 10f64.powf(-digits.max(0.0)) / 2.0;
            if !((act - expected).abs() < prec) {
                return fail(
                    format!(
                        "expect(received).toBeCloseTo({expected}, {digits})\n\nReceived: {act}"
                    ),
                    actual_close.clone(),
                    Some(Value::Number(expected)),
                    "toBeCloseTo",
                );
            }
            Value::Null
        }),
    );
    let actual_match_obj = actual.clone();
    m.insert(
        Arc::from("toMatchObject"),
        Value::native(move |args: &[Value]| {
            let expected = args.first().cloned().unwrap_or(Value::Null);
            if !crate::deep_equal::partial_deep_strict_equal(&actual_match_obj, &expected) {
                return fail(
                    format!(
                        "expect(received).toMatchObject(expected)\n\nExpected: {}\nReceived: {}",
                        expected.to_display_string(),
                        actual_match_obj.to_display_string()
                    ),
                    actual_match_obj.clone(),
                    Some(expected),
                    "toMatchObject",
                );
            }
            Value::Null
        }),
    );
    m.insert(
        Arc::from("toHaveLength"),
        Value::native(move |args: &[Value]| {
            let expected = args.first().and_then(|v| v.as_number()).unwrap_or(0.0);
            match length_of(&actual_len) {
                Some(n) if (n - expected).abs() < f64::EPSILON => Value::Null,
                Some(n) => fail(
                    format!(
                        "expect(received).toHaveLength({})\n\nReceived length: {}",
                        expected, n
                    ),
                    actual_len.clone(),
                    Some(Value::Number(expected)),
                    "toHaveLength",
                ),
                None => fail(
                    format!(
                        "expect(received).toHaveLength()\n\nReceived value has no length: {}",
                        actual_len.to_display_string()
                    ),
                    actual_len.clone(),
                    Some(Value::Number(expected)),
                    "toHaveLength",
                ),
            }
        }),
    );
    m.insert(
        Arc::from("toBeGreaterThan"),
        Value::native(move |args: &[Value]| {
            let exp = args.first().and_then(|v| v.as_number()).unwrap_or(0.0);
            let act = actual_gt.as_number().unwrap_or(f64::NAN);
            if !(act > exp) {
                return fail(
                    format!("expect({act}).toBeGreaterThan({exp})"),
                    actual_gt.clone(),
                    Some(Value::Number(exp)),
                    "toBeGreaterThan",
                );
            }
            Value::Null
        }),
    );
    m.insert(
        Arc::from("toBeGreaterThanOrEqual"),
        Value::native(move |args: &[Value]| {
            let exp = args.first().and_then(|v| v.as_number()).unwrap_or(0.0);
            let act = actual_gte.as_number().unwrap_or(f64::NAN);
            if !(act >= exp) {
                return fail(
                    format!("expect({act}).toBeGreaterThanOrEqual({exp})"),
                    actual_gte.clone(),
                    Some(Value::Number(exp)),
                    "toBeGreaterThanOrEqual",
                );
            }
            Value::Null
        }),
    );
    m.insert(
        Arc::from("toBeLessThan"),
        Value::native(move |args: &[Value]| {
            let exp = args.first().and_then(|v| v.as_number()).unwrap_or(0.0);
            let act = actual_lt.as_number().unwrap_or(f64::NAN);
            if !(act < exp) {
                return fail(
                    format!("expect({act}).toBeLessThan({exp})"),
                    actual_lt.clone(),
                    Some(Value::Number(exp)),
                    "toBeLessThan",
                );
            }
            Value::Null
        }),
    );
    m.insert(
        Arc::from("toBeLessThanOrEqual"),
        Value::native(move |args: &[Value]| {
            let exp = args.first().and_then(|v| v.as_number()).unwrap_or(0.0);
            let act = actual_lte.as_number().unwrap_or(f64::NAN);
            if !(act <= exp) {
                return fail(
                    format!("expect({act}).toBeLessThanOrEqual({exp})"),
                    actual_lte.clone(),
                    Some(Value::Number(exp)),
                    "toBeLessThanOrEqual",
                );
            }
            Value::Null
        }),
    );
    m.insert(
        Arc::from("toHaveBeenCalled"),
        Value::native(move |_args: &[Value]| {
            let Some(meta) = mock_meta(&actual_called) else {
                return fail(
                    "expect(...).toHaveBeenCalled() requires a mock or spy".into(),
                    actual_called.clone(),
                    None,
                    "toHaveBeenCalled",
                );
            };
            if mock_call_count(&meta) < 1.0 {
                return fail(
                    "expect(mock).toHaveBeenCalled()".into(),
                    actual_called.clone(),
                    None,
                    "toHaveBeenCalled",
                );
            }
            Value::Null
        }),
    );
    m.insert(
        Arc::from("toHaveBeenCalledTimes"),
        Value::native(move |args: &[Value]| {
            let Some(meta) = mock_meta(&actual_called_times) else {
                return fail(
                    "expect(...).toHaveBeenCalledTimes() requires a mock or spy".into(),
                    actual_called_times.clone(),
                    None,
                    "toHaveBeenCalledTimes",
                );
            };
            let expected = args.first().and_then(|v| v.as_number()).unwrap_or(0.0);
            let got = mock_call_count(&meta);
            if got != expected {
                return fail(
                    format!("expect(mock).toHaveBeenCalledTimes({expected})\n\nReceived: {got}"),
                    actual_called_times.clone(),
                    Some(Value::Number(expected)),
                    "toHaveBeenCalledTimes",
                );
            }
            Value::Null
        }),
    );
    m.insert(
        Arc::from("toHaveBeenCalledWith"),
        Value::native(move |args: &[Value]| {
            let Some(meta) = mock_meta(&actual_called_with) else {
                return fail(
                    "expect(...).toHaveBeenCalledWith() requires a mock or spy".into(),
                    actual_called_with.clone(),
                    None,
                    "toHaveBeenCalledWith",
                );
            };
            let expected = args.to_vec();
            let ok = mock_calls(&meta).iter().any(|call| match call {
                Value::Array(a) => {
                    let got = a.borrow();
                    got.len() == expected.len()
                        && got
                            .iter()
                            .zip(expected.iter())
                            .all(|(g, e)| deep_strict_equal(g, e))
                }
                _ => false,
            });
            if !ok {
                return fail(
                    format!(
                        "expect(mock).toHaveBeenCalledWith({})\n\nNumber of calls: {}",
                        expected
                            .iter()
                            .map(|v| v.to_display_string())
                            .collect::<Vec<_>>()
                            .join(", "),
                        mock_call_count(&meta)
                    ),
                    actual_called_with.clone(),
                    None,
                    "toHaveBeenCalledWith",
                );
            }
            Value::Null
        }),
    );
    m.insert(
        Arc::from("toHaveBeenLastCalledWith"),
        Value::native(move |args: &[Value]| {
            let Some(meta) = mock_meta(&actual_last_with) else {
                return fail(
                    "expect(...).toHaveBeenLastCalledWith() requires a mock or spy".into(),
                    actual_last_with.clone(),
                    None,
                    "toHaveBeenLastCalledWith",
                );
            };
            let expected = args.to_vec();
            let calls = mock_calls(&meta);
            let Some(last) = calls.last() else {
                return fail(
                    "expect(mock).toHaveBeenLastCalledWith(...)\n\nNumber of calls: 0".into(),
                    actual_last_with.clone(),
                    None,
                    "toHaveBeenLastCalledWith",
                );
            };
            let ok = match last {
                Value::Array(a) => {
                    let got = a.borrow();
                    got.len() == expected.len()
                        && got
                            .iter()
                            .zip(expected.iter())
                            .all(|(g, e)| deep_strict_equal(g, e))
                }
                _ => false,
            };
            if !ok {
                return fail(
                    "expect(mock).toHaveBeenLastCalledWith(...)".into(),
                    actual_last_with.clone(),
                    None,
                    "toHaveBeenLastCalledWith",
                );
            }
            Value::Null
        }),
    );
    m.insert(
        Arc::from("toMatchSnapshot"),
        Value::native(move |args: &[Value]| {
            let hint = args.first().map(|v| v.to_display_string());
            crate::snapshots::match_snapshot(&actual_snap, hint.as_deref())
        }),
    );
    let actual_nan = actual.clone();
    m.insert(
        Arc::from("toBeNaN"),
        Value::native(move |_args: &[Value]| {
            if !matches!(actual_nan, Value::Number(n) if n.is_nan()) {
                return fail(
                    format!(
                        "expect(received).toBeNaN()\n\nReceived: {}",
                        actual_nan.to_display_string()
                    ),
                    actual_nan.clone(),
                    None,
                    "toBeNaN",
                );
            }
            Value::Null
        }),
    );
    let actual_typeof = actual.clone();
    m.insert(
        Arc::from("toBeTypeOf"),
        Value::native(move |args: &[Value]| {
            let want = args
                .first()
                .map(|v| v.to_display_string())
                .unwrap_or_default();
            let got = type_of_name(&actual_typeof);
            if got != want {
                return fail(
                    format!("expect(received).toBeTypeOf({want:?})\n\nReceived type: {got}"),
                    actual_typeof.clone(),
                    None,
                    "toBeTypeOf",
                );
            }
            Value::Null
        }),
    );
    let actual_prop = actual.clone();
    m.insert(
        Arc::from("toHaveProperty"),
        Value::native(move |args: &[Value]| {
            let path = args.first().cloned().unwrap_or(Value::Null);
            let Some(found) = property_at(&actual_prop, &path) else {
                return fail(
                    format!(
                        "expect(received).toHaveProperty({})\n\nProperty not found on: {}",
                        path.to_display_string(),
                        actual_prop.to_display_string()
                    ),
                    actual_prop.clone(),
                    None,
                    "toHaveProperty",
                );
            };
            if let Some(expected) = args.get(1) {
                if !deep_strict_equal(&found, expected) {
                    return fail(
                        format!(
                            "expect(received).toHaveProperty({}, expected)\n\nExpected: {}\nReceived: {}",
                            path.to_display_string(),
                            expected.to_display_string(),
                            found.to_display_string()
                        ),
                        found,
                        Some(expected.clone()),
                        "toHaveProperty",
                    );
                }
            }
            Value::Null
        }),
    );
    let actual_contain_eq = actual.clone();
    m.insert(
        Arc::from("toContainEqual"),
        Value::native(move |args: &[Value]| {
            let item = args.first().cloned().unwrap_or(Value::Null);
            let items = match actual_contain_eq.clone().coerce_number_array() {
                Value::Array(a) => a.borrow().clone(),
                _ => Vec::new(),
            };
            if !items.iter().any(|v| deep_strict_equal(v, &item)) {
                return fail(
                    format!(
                        "expect(received).toContainEqual(expected)\n\nExpected to contain: {}\nReceived: {}",
                        item.to_display_string(),
                        actual_contain_eq.to_display_string()
                    ),
                    actual_contain_eq.clone(),
                    Some(item),
                    "toContainEqual",
                );
            }
            Value::Null
        }),
    );

    // `toThrowError` is Jest's alias for `toThrow`.
    if let Some(t) = m.get("toThrow").cloned() {
        m.insert(Arc::from("toThrowError"), t);
    }

    attach_not(&mut m, actual);
    Value::object(m)
}

/// Build `expect(x).not.*` by inverting each matcher via pending-throw polarity.
fn attach_not(m: &mut ObjectMap, actual: Value) {
    let mut not_map = ObjectMap::default();
    let keys: Vec<Arc<str>> = m.keys().cloned().collect();
    for key in keys {
        // `.not.toMatchSnapshot()` would write (or diff-invert) a snapshot; Jest has no such
        // matcher either.
        if key.as_ref() == "not" || key.as_ref() == "toMatchSnapshot" {
            continue;
        }
        let Some(matcher) = m.get(key.as_ref()).cloned() else {
            continue;
        };
        let act = actual.clone();
        let name = key.to_string();
        not_map.insert(
            Arc::clone(&key),
            Value::native(move |args: &[Value]| {
                let _ = take_pending_throw();
                let _ = value_call(&matcher, args);
                if has_pending_throw() {
                    let _ = take_pending_throw();
                    return Value::Null;
                }
                fail(
                    format!("expect(received).not.{name}() expected matcher to fail"),
                    act.clone(),
                    None,
                    &format!("not.{name}"),
                )
            }),
        );
    }
    m.insert(Arc::from("not"), Value::object(not_map));
}

thread_local! {
    static ASSERTIONS_MADE: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// From `expect.assertions(n)`.
    static ASSERTIONS_EXPECTED: std::cell::Cell<Option<usize>> =
        const { std::cell::Cell::new(None) };
    /// From `expect.hasAssertions()` — "at least one".
    static ASSERTIONS_AT_LEAST: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Reset the per-test assertion counters. Called by the runner before each test body.
pub fn begin_test_assertions() {
    ASSERTIONS_MADE.with(|c| c.set(0));
    ASSERTIONS_EXPECTED.with(|c| c.set(None));
    ASSERTIONS_AT_LEAST.with(|c| c.set(false));
}

/// Verify an `expect.assertions(n)` / `expect.hasAssertions()` contract for the test that just ran.
pub fn check_test_assertions() -> Result<(), String> {
    let made = ASSERTIONS_MADE.with(|c| c.get());
    if ASSERTIONS_AT_LEAST.with(|c| c.get()) && made == 0 {
        return Err("expect.hasAssertions(): no assertions were made".to_string());
    }
    if let Some(want) = ASSERTIONS_EXPECTED.with(|c| c.get()) {
        if made != want {
            return Err(format!(
                "expect.assertions({want}): {made} assertion(s) were made"
            ));
        }
    }
    Ok(())
}

fn expect_call(args: &[Value]) -> Value {
    ASSERTIONS_MADE.with(|c| c.set(c.get() + 1));
    let actual = args.first().cloned().unwrap_or(Value::Null);
    matcher_object(actual)
}

fn asymmetric_any(args: &[Value]) -> Value {
    let type_name = args
        .first()
        .map(|v| v.to_display_string())
        .unwrap_or_else(|| "Object".into());
    let mut m = ObjectMap::default();
    m.insert(Arc::from(ASYMMETRIC_KEY), Value::String("any".into()));
    m.insert(Arc::from("typeName"), Value::String(type_name.into()));
    Value::object(m)
}

fn asymmetric_object_containing(args: &[Value]) -> Value {
    let sample = args.first().cloned().unwrap_or(Value::Null);
    let mut m = ObjectMap::default();
    m.insert(
        Arc::from(ASYMMETRIC_KEY),
        Value::String("objectContaining".into()),
    );
    m.insert(Arc::from("sample"), sample);
    Value::object(m)
}

fn asymmetric_array_containing(args: &[Value]) -> Value {
    let sample = args.first().cloned().unwrap_or(Value::Null);
    let mut m = ObjectMap::default();
    m.insert(
        Arc::from(ASYMMETRIC_KEY),
        Value::String("arrayContaining".into()),
    );
    m.insert(Arc::from("sample"), sample);
    Value::object(m)
}

fn asymmetric_string_matching(args: &[Value]) -> Value {
    let sample = args.first().cloned().unwrap_or(Value::Null);
    let mut m = ObjectMap::default();
    m.insert(
        Arc::from(ASYMMETRIC_KEY),
        Value::String("stringMatching".into()),
    );
    m.insert(Arc::from("sample"), sample);
    Value::object(m)
}

/// Marker key for `expect.any` / `objectContaining` / … Deliberately *not* React's `$$typeof`,
/// which a plain user object may legitimately carry — treating those as matchers made them
/// compare unequal to everything.
pub(crate) const ASYMMETRIC_KEY: &str = "__tishAsymmetricMatcher";

/// Build the callable `expect` object.
pub fn expect_object() -> Value {
    let mut m = ObjectMap::default();
    m.insert(Arc::from("__call"), Value::native(expect_call));
    m.insert(
        Arc::from("assertions"),
        Value::native(|args: &[Value]| {
            let n = args.first().and_then(|v| v.as_number()).unwrap_or(0.0);
            ASSERTIONS_EXPECTED.with(|c| c.set(Some(n.max(0.0) as usize)));
            Value::Null
        }),
    );
    m.insert(
        Arc::from("hasAssertions"),
        Value::native(|_args: &[Value]| {
            ASSERTIONS_AT_LEAST.with(|c| c.set(true));
            Value::Null
        }),
    );
    m.insert(Arc::from("any"), Value::native(asymmetric_any));
    m.insert(
        Arc::from("anything"),
        Value::native(|_args: &[Value]| {
            let mut o = ObjectMap::default();
            o.insert(Arc::from(ASYMMETRIC_KEY), Value::String("anything".into()));
            Value::object(o)
        }),
    );
    m.insert(
        Arc::from("objectContaining"),
        Value::native(asymmetric_object_containing),
    );
    m.insert(
        Arc::from("arrayContaining"),
        Value::native(asymmetric_array_containing),
    );
    m.insert(
        Arc::from("stringMatching"),
        Value::native(asymmetric_string_matching),
    );
    Value::object(m)
}
