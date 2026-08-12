//! Shared deep strict equality for `expect.toEqual` and `assert.deepStrictEqual`.

use std::collections::HashSet;

use tishlang_core::{ObjectData, Value, VmRef};

fn same_value_zero_num(a: f64, b: f64) -> bool {
    if a.is_nan() && b.is_nan() {
        return true;
    }
    if a == 0.0 && b == 0.0 {
        return true;
    }
    a == b
}

fn ptr_key_obj(o: &VmRef<ObjectData>) -> usize {
    o.as_ptr() as usize
}

fn ptr_key_arr(a: &VmRef<Vec<Value>>) -> usize {
    a.as_ptr() as usize
}

fn type_name_of(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "Boolean",
        Value::Number(_) => "Number",
        Value::String(_) => "String",
        Value::Symbol(_) => "Symbol",
        Value::Array(_) | Value::NumberArray(_) => "Array",
        Value::Object(_) => "Object",
        Value::Function(_) => "Function",
        #[cfg(feature = "regex")]
        Value::RegExp(_) => "RegExp",
        Value::Promise(_) => "Promise",
        Value::Opaque(_) => "Object",
    }
}

/// Jest-style asymmetric matchers (`expect.any`, `objectContaining`, …) on the expected side.
fn asymmetric_match(
    actual: &Value,
    expected: &Value,
    seen: &mut HashSet<(usize, usize)>,
) -> Option<bool> {
    let Value::Object(e) = expected else {
        return None;
    };
    let eb = e.borrow();
    let Some(Value::String(kind)) = eb.strings.get(crate::expect::ASYMMETRIC_KEY) else {
        return None;
    };
    Some(match kind.as_str() {
        "anything" => !matches!(actual, Value::Null),
        "any" => {
            let want = eb
                .strings
                .get("typeName")
                .map(|v| v.to_display_string())
                .unwrap_or_else(|| "Object".into());
            let got = type_name_of(actual);
            want.eq_ignore_ascii_case(got)
                || (want.eq_ignore_ascii_case("Object") && matches!(actual, Value::Object(_)))
        }
        "objectContaining" => {
            let sample = eb.strings.get("sample").cloned().unwrap_or(Value::Null);
            partial_deep_strict_equal(actual, &sample)
        }
        "arrayContaining" => {
            // The sample needs the same packed-array materialization as `actual`; a numeric
            // literal like `[1, 2]` can arrive as a `NumberArray`.
            let sample = eb
                .strings
                .get("sample")
                .cloned()
                .unwrap_or(Value::Null)
                .coerce_number_array();
            let Value::Array(want) = sample else {
                return Some(false);
            };
            let actual_coerced = actual.clone().coerce_number_array();
            let Value::Array(haystack) = actual_coerced else {
                return Some(false);
            };
            let needles: Vec<Value> = want.borrow().clone();
            let hay: Vec<Value> = haystack.borrow().clone();
            needles.iter().all(|needle| {
                hay.iter()
                    .any(|item| deep_strict_equal_inner(item, needle, seen))
            })
        }
        "stringMatching" => {
            let sample = eb.strings.get("sample").cloned().unwrap_or(Value::Null);
            let Value::String(s) = actual else {
                return Some(false);
            };
            match sample {
                Value::String(pat) => s.contains(pat.as_str()),
                #[cfg(feature = "regex")]
                Value::RegExp(re) => re.borrow_mut().test(s),
                _ => false,
            }
        }
        _ => false,
    })
}

/// Deep strict equality with cycle detection.
pub fn deep_strict_equal(a: &Value, b: &Value) -> bool {
    let mut seen = HashSet::new();
    deep_strict_equal_inner(a, b, &mut seen)
}

fn deep_strict_equal_inner(a: &Value, b: &Value, seen: &mut HashSet<(usize, usize)>) -> bool {
    // Materialize packed number arrays to generic arrays for structural compare.
    let a = a.clone().coerce_number_array();
    let b = b.clone().coerce_number_array();

    if let Some(ok) = asymmetric_match(&a, &b, seen) {
        return ok;
    }
    if let Some(ok) = asymmetric_match(&b, &a, seen) {
        return ok;
    }

    match (&a, &b) {
        (Value::Null, Value::Null) => true,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::Number(x), Value::Number(y)) => same_value_zero_num(*x, *y),
        (Value::String(x), Value::String(y)) => x.as_str() == y.as_str(),
        (Value::Symbol(x), Value::Symbol(y)) => std::sync::Arc::ptr_eq(x, y),
        (Value::Array(x), Value::Array(y)) => {
            if VmRef::ptr_eq(x, y) {
                return true;
            }
            let kx = ptr_key_arr(x);
            let ky = ptr_key_arr(y);
            let pair = (kx.min(ky), kx.max(ky));
            if !seen.insert(pair) {
                return true;
            }
            let xb = x.borrow();
            let yb = y.borrow();
            if xb.len() != yb.len() {
                return false;
            }
            xb.iter()
                .zip(yb.iter())
                .all(|(u, v)| deep_strict_equal_inner(u, v, seen))
        }
        (Value::Object(x), Value::Object(y)) => {
            if VmRef::ptr_eq(x, y) {
                return true;
            }
            let kx = ptr_key_obj(x);
            let ky = ptr_key_obj(y);
            let pair = (kx.min(ky), kx.max(ky));
            if !seen.insert(pair) {
                return true;
            }
            let xb = x.borrow();
            let yb = y.borrow();
            if xb.strings.len() != yb.strings.len() {
                return false;
            }
            for (k, v) in xb.strings.iter() {
                match yb.strings.get(k) {
                    Some(ov) => {
                        if !deep_strict_equal_inner(v, ov, seen) {
                            return false;
                        }
                    }
                    None => return false,
                }
            }
            true
        }
        #[cfg(feature = "regex")]
        (Value::RegExp(x), Value::RegExp(y)) => {
            let xb = x.borrow();
            let yb = y.borrow();
            xb.source == yb.source && format!("{}", xb.flags) == format!("{}", yb.flags)
        }
        // Jest compares functions by identity: `expect(fn).toEqual(fn)` passes, two distinct
        // functions never do. Returning false unconditionally failed the identity case.
        (Value::Function(x), Value::Function(y)) => std::sync::Arc::ptr_eq(x, y),
        (Value::Opaque(x), Value::Opaque(y)) => std::sync::Arc::ptr_eq(x, y),
        _ => false,
    }
}

/// Partial deep strict: every key in `expected` exists in `actual` with matching values.
pub fn partial_deep_strict_equal(actual: &Value, expected: &Value) -> bool {
    match (actual, expected) {
        (Value::Object(a), Value::Object(e)) => {
            let ab = a.borrow();
            let eb = e.borrow();
            for (k, ev) in eb.strings.iter() {
                match ab.strings.get(k) {
                    Some(av) => {
                        if matches!(ev, Value::Object(_) | Value::Array(_)) {
                            if !partial_deep_strict_equal(av, ev) {
                                return false;
                            }
                        } else if !deep_strict_equal(av, ev) {
                            return false;
                        }
                    }
                    None => return false,
                }
            }
            true
        }
        (Value::Array(a), Value::Array(e)) => {
            let ab = a.borrow();
            let eb = e.borrow();
            if eb.len() > ab.len() {
                return false;
            }
            for (ev, av) in eb.iter().zip(ab.iter()) {
                if matches!(ev, Value::Object(_) | Value::Array(_)) {
                    if !partial_deep_strict_equal(av, ev) {
                        return false;
                    }
                } else if !deep_strict_equal(av, ev) {
                    return false;
                }
            }
            true
        }
        _ => deep_strict_equal(actual, expected),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tishlang_core::{Arc, ObjectMap};

    #[test]
    fn primitives() {
        assert!(deep_strict_equal(&Value::Null, &Value::Null));
        assert!(deep_strict_equal(&Value::Number(1.0), &Value::Number(1.0)));
        assert!(deep_strict_equal(
            &Value::Number(f64::NAN),
            &Value::Number(f64::NAN)
        ));
        assert!(!deep_strict_equal(&Value::Number(1.0), &Value::Number(2.0)));
    }

    #[test]
    fn objects() {
        let mut m1 = ObjectMap::default();
        m1.insert(Arc::from("a"), Value::Number(1.0));
        let mut m2 = ObjectMap::default();
        m2.insert(Arc::from("a"), Value::Number(1.0));
        assert!(deep_strict_equal(&Value::object(m1), &Value::object(m2)));
    }
}
