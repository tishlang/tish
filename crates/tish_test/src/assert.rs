//! Node-compatible `assert` / `tish:assert` surface.

use tishlang_core::{
    has_pending_throw, set_pending_throw, take_pending_throw, value_call, Arc, ObjectMap, Value,
};

use crate::deep_equal::{deep_strict_equal, partial_deep_strict_equal};

pub fn assertion_error(
    message: impl Into<String>,
    actual: Option<Value>,
    expected: Option<Value>,
    operator: &str,
    generated_message: bool,
) -> Value {
    let msg = message.into();
    let mut m = ObjectMap::default();
    m.insert(Arc::from("name"), Value::String("AssertionError".into()));
    m.insert(Arc::from("message"), Value::String(msg.into()));
    m.insert(Arc::from("code"), Value::String("ERR_ASSERTION".into()));
    m.insert(Arc::from("operator"), Value::String(operator.into()));
    m.insert(
        Arc::from("generatedMessage"),
        Value::Bool(generated_message),
    );
    if let Some(a) = actual {
        m.insert(Arc::from("actual"), a);
    }
    if let Some(e) = expected {
        m.insert(Arc::from("expected"), e);
    }
    Value::object(m)
}

fn throw_err(
    message: impl Into<String>,
    actual: Option<Value>,
    expected: Option<Value>,
    operator: &str,
) -> Value {
    set_pending_throw(assertion_error(message, actual, expected, operator, true));
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

fn ok(args: &[Value]) -> Value {
    let v = args.first().cloned().unwrap_or(Value::Null);
    let msg = args
        .get(1)
        .map(|m| m.to_display_string())
        .unwrap_or_else(|| "The expression evaluated to a falsy value".into());
    if !is_truthy(&v) {
        return throw_err(msg, Some(v), Some(Value::Bool(true)), "==");
    }
    Value::Null
}

fn strict_equal(args: &[Value]) -> Value {
    let actual = args.first().cloned().unwrap_or(Value::Null);
    let expected = args.get(1).cloned().unwrap_or(Value::Null);
    let msg = args.get(2).map(|m| m.to_display_string());
    if !actual.strict_eq(&expected) {
        let m = msg.unwrap_or_else(|| {
            format!(
                "Expected values to be strictly equal:\n+ actual: {}\n- expected: {}",
                actual.to_display_string(),
                expected.to_display_string()
            )
        });
        return throw_err(m, Some(actual), Some(expected), "strictEqual");
    }
    Value::Null
}

fn not_strict_equal(args: &[Value]) -> Value {
    let actual = args.first().cloned().unwrap_or(Value::Null);
    let expected = args.get(1).cloned().unwrap_or(Value::Null);
    let msg = args.get(2).map(|m| m.to_display_string());
    if actual.strict_eq(&expected) {
        let m = msg.unwrap_or_else(|| {
            format!(
                "Expected values to be strictly unequal:\nActual: {}",
                actual.to_display_string()
            )
        });
        return throw_err(m, Some(actual), Some(expected), "notStrictEqual");
    }
    Value::Null
}

fn deep_strict(args: &[Value]) -> Value {
    let actual = args.first().cloned().unwrap_or(Value::Null);
    let expected = args.get(1).cloned().unwrap_or(Value::Null);
    let msg = args.get(2).map(|m| m.to_display_string());
    if !deep_strict_equal(&actual, &expected) {
        let m = msg.unwrap_or_else(|| {
            format!(
                "Expected values to be strictly deep-equal:\n+ actual: {}\n- expected: {}",
                actual.to_display_string(),
                expected.to_display_string()
            )
        });
        return throw_err(m, Some(actual), Some(expected), "deepStrictEqual");
    }
    Value::Null
}

fn not_deep_strict(args: &[Value]) -> Value {
    let actual = args.first().cloned().unwrap_or(Value::Null);
    let expected = args.get(1).cloned().unwrap_or(Value::Null);
    let msg = args.get(2).map(|m| m.to_display_string());
    if deep_strict_equal(&actual, &expected) {
        let m = msg.unwrap_or_else(|| {
            format!(
                "Expected values not to be strictly deep-equal:\nActual: {}",
                actual.to_display_string()
            )
        });
        return throw_err(m, Some(actual), Some(expected), "notDeepStrictEqual");
    }
    Value::Null
}

fn partial_deep(args: &[Value]) -> Value {
    let actual = args.first().cloned().unwrap_or(Value::Null);
    let expected = args.get(1).cloned().unwrap_or(Value::Null);
    let msg = args.get(2).map(|m| m.to_display_string());
    if !partial_deep_strict_equal(&actual, &expected) {
        let m = msg.unwrap_or_else(|| {
            format!(
                "Expected object to match partial:\n+ actual: {}\n- expected: {}",
                actual.to_display_string(),
                expected.to_display_string()
            )
        });
        return throw_err(m, Some(actual), Some(expected), "partialDeepStrictEqual");
    }
    Value::Null
}

fn match_fn(args: &[Value]) -> Value {
    let string = args
        .first()
        .map(|v| v.to_display_string())
        .unwrap_or_default();
    let regexp = args.get(1).cloned().unwrap_or(Value::Null);
    let msg = args.get(2).map(|m| m.to_display_string());
    let ok = regexp_test(&regexp, &string);
    if !ok {
        let m = msg.unwrap_or_else(|| {
            format!(
                "The input did not match the regular expression {}:\n{}",
                regexp.to_display_string(),
                string
            )
        });
        return throw_err(
            m,
            Some(Value::String(string.into())),
            Some(regexp),
            "match",
        );
    }
    Value::Null
}

fn does_not_match(args: &[Value]) -> Value {
    let string = args
        .first()
        .map(|v| v.to_display_string())
        .unwrap_or_default();
    let regexp = args.get(1).cloned().unwrap_or(Value::Null);
    let msg = args.get(2).map(|m| m.to_display_string());
    if regexp_test(&regexp, &string) {
        let m = msg.unwrap_or_else(|| {
            format!(
                "The input was expected to not match {}:\n{}",
                regexp.to_display_string(),
                string
            )
        });
        return throw_err(
            m,
            Some(Value::String(string.into())),
            Some(regexp),
            "doesNotMatch",
        );
    }
    Value::Null
}

fn regexp_test(re: &Value, s: &str) -> bool {
    #[cfg(feature = "regex")]
    {
        if let Value::RegExp(r) = re {
            return r.borrow_mut().test(s);
        }
    }
    let pat = re.to_display_string();
    regex::Regex::new(&pat)
        .map(|r| r.is_match(s))
        .unwrap_or(false)
}

fn throws(args: &[Value]) -> Value {
    let fn_v = args.first().cloned().unwrap_or(Value::Null);
    let _ = take_pending_throw();
    let _ = value_call(&fn_v, &[]);
    if has_pending_throw() {
        let _ = take_pending_throw();
        return Value::Null;
    }
    let msg = args
        .get(1)
        .map(|m| m.to_display_string())
        .unwrap_or_else(|| "Missing expected exception".into());
    throw_err(msg, None, None, "throws")
}

fn does_not_throw(args: &[Value]) -> Value {
    let fn_v = args.first().cloned().unwrap_or(Value::Null);
    let _ = take_pending_throw();
    let _ = value_call(&fn_v, &[]);
    if has_pending_throw() {
        let err = take_pending_throw().unwrap_or(Value::Null);
        let msg = args
            .get(1)
            .map(|m| m.to_display_string())
            .unwrap_or_else(|| format!("Got unwanted exception: {}", err.to_display_string()));
        return throw_err(msg, Some(err), None, "doesNotThrow");
    }
    Value::Null
}

fn rejects(args: &[Value]) -> Value {
    let p = args.first().cloned().unwrap_or(Value::Null);
    #[cfg(feature = "promise")]
    {
        let _ = take_pending_throw();
        let settled = match p {
            Value::Promise(pr) => pr.block_until_settled(),
            other => Ok(other),
        };
        match settled {
            Err(_) => Value::Null,
            Ok(_) => {
                let msg = args
                    .get(1)
                    .map(|m| m.to_display_string())
                    .unwrap_or_else(|| "Missing expected rejection".into());
                throw_err(msg, None, None, "rejects")
            }
        }
    }
    #[cfg(not(feature = "promise"))]
    {
        let _ = p;
        throw_err(
            "assert.rejects requires the promise feature",
            None,
            None,
            "rejects",
        )
    }
}

fn does_not_reject(args: &[Value]) -> Value {
    let p = args.first().cloned().unwrap_or(Value::Null);
    #[cfg(feature = "promise")]
    {
        let _ = take_pending_throw();
        let settled = match p {
            Value::Promise(pr) => pr.block_until_settled(),
            other => Ok(other),
        };
        match settled {
            Ok(_) => Value::Null,
            Err(err) => {
                let msg = args
                    .get(1)
                    .map(|m| m.to_display_string())
                    .unwrap_or_else(|| {
                        format!("Got unwanted rejection: {}", err.to_display_string())
                    });
                throw_err(msg, Some(err), None, "doesNotReject")
            }
        }
    }
    #[cfg(not(feature = "promise"))]
    {
        let _ = p;
        throw_err(
            "assert.doesNotReject requires the promise feature",
            None,
            None,
            "doesNotReject",
        )
    }
}

fn fail(args: &[Value]) -> Value {
    let msg = args
        .first()
        .map(|m| m.to_display_string())
        .unwrap_or_else(|| "Failed".into());
    throw_err(msg, None, None, "fail")
}

fn if_error(args: &[Value]) -> Value {
    let v = args.first().cloned().unwrap_or(Value::Null);
    if matches!(v, Value::Null) {
        return Value::Null;
    }
    // Treat non-null as error
    throw_err(
        format!("ifError got unwanted exception: {}", v.to_display_string()),
        Some(v),
        Some(Value::Null),
        "ifError",
    )
}

/// Build the callable `assert` object (`assert(x)` + methods).
pub fn assert_object() -> Value {
    let call = Value::native(ok);
    let mut m = ObjectMap::default();
    m.insert(Arc::from("__call"), call);
    m.insert(Arc::from("ok"), Value::native(ok));
    m.insert(Arc::from("equal"), Value::native(strict_equal));
    m.insert(Arc::from("notEqual"), Value::native(not_strict_equal));
    m.insert(Arc::from("strictEqual"), Value::native(strict_equal));
    m.insert(Arc::from("notStrictEqual"), Value::native(not_strict_equal));
    m.insert(Arc::from("deepEqual"), Value::native(deep_strict));
    m.insert(Arc::from("notDeepEqual"), Value::native(not_deep_strict));
    m.insert(Arc::from("deepStrictEqual"), Value::native(deep_strict));
    m.insert(Arc::from("notDeepStrictEqual"), Value::native(not_deep_strict));
    m.insert(Arc::from("partialDeepStrictEqual"), Value::native(partial_deep));
    m.insert(Arc::from("match"), Value::native(match_fn));
    m.insert(Arc::from("doesNotMatch"), Value::native(does_not_match));
    m.insert(Arc::from("throws"), Value::native(throws));
    m.insert(Arc::from("doesNotThrow"), Value::native(does_not_throw));
    m.insert(Arc::from("rejects"), Value::native(rejects));
    m.insert(Arc::from("doesNotReject"), Value::native(does_not_reject));
    m.insert(Arc::from("fail"), Value::native(fail));
    m.insert(Arc::from("ifError"), Value::native(if_error));
    m.insert(
        Arc::from("AssertionError"),
        Value::native(|args: &[Value]| {
            let msg = args
                .first()
                .map(|v| v.to_display_string())
                .unwrap_or_default();
            assertion_error(msg, None, None, "fail", false)
        }),
    );
    Value::object(m)
}
