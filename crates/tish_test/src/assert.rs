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
        return throw_err(m, Some(Value::String(string.into())), Some(regexp), "match");
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

/// Node's `assert.throws(fn[, error][, message])`: `error` is a validator (regex / message
/// substring / `Error`-shaped object), `message` is the failure text.
fn split_error_and_message(args: &[Value]) -> (Option<Value>, Option<String>) {
    match (args.get(1), args.get(2)) {
        (Some(e), Some(m)) => (Some(e.clone()), Some(m.to_display_string())),
        (Some(e), None) => (Some(e.clone()), None),
        _ => (None, None),
    }
}

fn throws(args: &[Value]) -> Value {
    let fn_v = args.first().cloned().unwrap_or(Value::Null);
    let (expected, message) = split_error_and_message(args);
    if !crate::expect::is_callable(&fn_v) {
        return throw_err(
            format!(
                "assert.throws expects a function, got {}",
                fn_v.to_display_string()
            ),
            Some(fn_v),
            None,
            "throws",
        );
    }
    let _ = take_pending_throw();
    let _ = value_call(&fn_v, &[]);
    if !has_pending_throw() {
        let msg = message.unwrap_or_else(|| "Missing expected exception".into());
        return throw_err(msg, None, expected, "throws");
    }
    let err = take_pending_throw().unwrap_or(Value::Null);
    if let Some(expected) = expected {
        // Previously the second argument was treated as the failure message, so ANY throw
        // satisfied `assert.throws(fn, /pattern/)`. Validate it.
        if !crate::expect::error_matches(&err, &expected) {
            let msg = message.unwrap_or_else(|| {
                format!(
                    "The error did not match the expected pattern:\n+ actual: {}\n- expected: {}",
                    err.to_display_string(),
                    expected.to_display_string()
                )
            });
            return throw_err(msg, Some(err), Some(expected), "throws");
        }
    }
    Value::Null
}

fn does_not_throw(args: &[Value]) -> Value {
    let fn_v = args.first().cloned().unwrap_or(Value::Null);
    if !crate::expect::is_callable(&fn_v) {
        return throw_err(
            format!(
                "assert.doesNotThrow expects a function, got {}",
                fn_v.to_display_string()
            ),
            Some(fn_v),
            None,
            "doesNotThrow",
        );
    }
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

/// Settle `assert.rejects` / `doesNotReject` input. Node accepts a promise **or a function
/// returning one**; the function form used to be treated as an already-resolved value, so
/// `assert.rejects(async () => { throw … })` never ran the body.
#[cfg(feature = "promise")]
fn settle_awaitable(v: Value) -> Result<Value, Value> {
    let v = if crate::expect::is_callable(&v) {
        let _ = take_pending_throw();
        let produced = value_call(&v, &[]);
        if has_pending_throw() {
            // A synchronous throw from the function counts as a rejection, as it does in Node.
            return Err(take_pending_throw().unwrap_or(Value::Null));
        }
        produced
    } else {
        v
    };
    match v {
        Value::Promise(pr) => pr.block_until_settled(),
        other => Ok(other),
    }
}

fn rejects(args: &[Value]) -> Value {
    let p = args.first().cloned().unwrap_or(Value::Null);
    #[cfg(feature = "promise")]
    {
        let (expected, message) = split_error_and_message(args);
        match settle_awaitable(p) {
            Err(err) => {
                if let Some(expected) = expected {
                    if !crate::expect::error_matches(&err, &expected) {
                        let msg = message.unwrap_or_else(|| {
                            format!(
                                "The rejection did not match the expected pattern:\n+ actual: {}\n- expected: {}",
                                err.to_display_string(),
                                expected.to_display_string()
                            )
                        });
                        return throw_err(msg, Some(err), Some(expected), "rejects");
                    }
                }
                Value::Null
            }
            Ok(_) => {
                let msg = message.unwrap_or_else(|| "Missing expected rejection".into());
                throw_err(msg, None, expected, "rejects")
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
        match settle_awaitable(p) {
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
    m.insert(
        Arc::from("notDeepStrictEqual"),
        Value::native(not_deep_strict),
    );
    m.insert(
        Arc::from("partialDeepStrictEqual"),
        Value::native(partial_deep),
    );
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
