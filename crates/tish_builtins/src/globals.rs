//! Global builtin functions with signature (args: &[Value]) -> Value.
//!
//! Used by both tishlang_vm (bytecode) and tishlang_runtime (compiled). Keeps tishlang_vm
//! independent of tishlang_runtime.

use std::sync::Arc;
use tishlang_core::VmRef;
use tishlang_core::{percent_decode, percent_encode, ObjectMap, Value};

/// Boolean(value) - coerce to bool
pub fn boolean(args: &[Value]) -> Value {
    let v = args.first().unwrap_or(&Value::Null);
    Value::Bool(v.is_truthy())
}

/// decodeURI(str)
pub fn decode_uri(args: &[Value]) -> Value {
    let s = args
        .first()
        .map(Value::to_display_string)
        .unwrap_or_default();
    Value::String(percent_decode(&s).unwrap_or(s).into())
}

/// encodeURI(str)
pub fn encode_uri(args: &[Value]) -> Value {
    let s = args
        .first()
        .map(Value::to_display_string)
        .unwrap_or_default();
    Value::String(percent_encode(&s).into())
}

/// isFinite(value) — coerces via ToNumber (like `Number()`), then tests finiteness. `isFinite("3")`
/// is `true`, `isFinite("x")`/`isFinite()` is `false`. (The non-coercing form is `Number.isFinite`.)
pub fn is_finite(args: &[Value]) -> Value {
    Value::Bool(args.first().map_or(f64::NAN, to_number).is_finite())
}

/// isNaN(value) — coerces via ToNumber, then tests NaN. `isNaN("3")` is `false`; an absent arg
/// (undefined) coerces to NaN → `true`. (The non-coercing form is `Number.isNaN`.)
pub fn is_nan(args: &[Value]) -> Value {
    Value::Bool(args.first().map_or(f64::NAN, to_number).is_nan())
}

/// Array.isArray(value)
pub fn array_is_array(args: &[Value]) -> Value {
    Value::Bool(matches!(args.first(), Some(Value::Array(_)) | Some(Value::NumberArray(_))))
}

/// `Array.of(...items)` — build an array from the args (unlike `Array(n)`, a single number is an
/// element, not a length: `Array.of(7)` is `[7]`).
pub fn array_of(args: &[Value]) -> Value {
    Value::Array(VmRef::new(args.to_vec()))
}

/// `Array.from(source, mapFn?)` — build an array from an iterable (array, string, Set/Map, or any
/// `__drain__`/`next` iterator — via the shared iterator protocol) or an array-like `{ length }`
/// object, optionally mapping each element through `mapFn(item, index)`.
pub fn array_from(args: &[Value]) -> Value {
    let source = args.first().cloned().unwrap_or(Value::Null);
    let items: Vec<Value> = match &source {
        Value::Array(a) => a.borrow().clone(),
        Value::NumberArray(a) => a.borrow().iter().map(|n| Value::Number(*n)).collect(),
        // `drain_iterator` covers String (chars), Set/Map, and any iterator object.
        other => {
            if let Some(d) = tishlang_core::drain_iterator(other) {
                d
            } else if matches!(other, Value::Object(_)) {
                let len = tishlang_core::object_get(other, &Value::String("length".into()))
                    .and_then(|v| match v {
                        Value::Number(n) if n.is_finite() && n >= 0.0 => Some(n as usize),
                        _ => None,
                    })
                    .unwrap_or(0);
                (0..len)
                    .map(|i| {
                        tishlang_core::object_get(other, &Value::String(i.to_string().into()))
                            .unwrap_or(Value::Null)
                    })
                    .collect()
            } else {
                Vec::new()
            }
        }
    };
    let out: Vec<Value> = match args.get(1) {
        Some(Value::Function(f)) => items
            .into_iter()
            .enumerate()
            .map(|(i, x)| f.call(&[x, Value::Number(i as f64)]))
            .collect(),
        _ => items,
    };
    Value::Array(VmRef::new(out))
}

/// `Object.is(a, b)` — SameValue: like `===`, but NaN equals NaN and `+0` is NOT equal to `-0`.
pub fn object_is(args: &[Value]) -> Value {
    let a = args.first().unwrap_or(&Value::Null);
    let b = args.get(1).unwrap_or(&Value::Null);
    let same = match (a, b) {
        (Value::Number(x), Value::Number(y)) => {
            if x.is_nan() && y.is_nan() {
                true
            } else if *x == 0.0 && *y == 0.0 {
                x.is_sign_negative() == y.is_sign_negative()
            } else {
                x == y
            }
        }
        _ => a.strict_eq(b),
    };
    Value::Bool(same)
}

/// String(value) — convert value to string (JS String constructor as function).
/// Uses JS `ToString` (arrays comma-join recursively, objects → "[object Object]"),
/// not the inspect/display form.
pub fn string_convert(args: &[Value]) -> Value {
    let v = args.first().unwrap_or(&Value::Null);
    Value::String(v.to_js_string().into())
}

/// JS `Number(value)` coercion (ToNumber), issue #36. Numbers pass through; booleans →
/// 1/0; null → 0; strings parse (trimmed, with `0x`/`0b`/`0o` and `Infinity`, `""` → 0,
/// otherwise NaN); arrays/objects go via their string form (so `Number([5])` → 5,
/// `Number([])` → 0, objects → NaN).
pub fn number_convert(args: &[Value]) -> Value {
    Value::Number(to_number(args.first().unwrap_or(&Value::Null)))
}

/// JS `ToNumber(v)`: Number → itself; Bool → 1/0; Null → 0; String → parsed (trimmed, `0x`/`0b`/`0o`
/// and `Infinity`; `""` → 0, else NaN); anything else via its string form. The single ToNumber path
/// shared by `Number()`, `isNaN`/`isFinite`, etc.
pub fn to_number(v: &Value) -> f64 {
    match v {
        Value::Number(n) => *n,
        Value::Bool(b) => {
            if *b {
                1.0
            } else {
                0.0
            }
        }
        Value::Null => 0.0,
        Value::String(s) => parse_numeric_string(s),
        other => parse_numeric_string(&other.to_js_string()),
    }
}

/// `Number.isInteger(v)` — true iff `v` IS a finite Number with no fractional part. No coercion
/// (`Number.isInteger("3")` is `false`, unlike a bare `isNaN`-style coercion).
pub fn number_is_integer(args: &[Value]) -> Value {
    Value::Bool(matches!(args.first(), Some(Value::Number(n)) if n.is_finite() && n.fract() == 0.0))
}

/// `Number.isSafeInteger(v)` — `isInteger` AND exactly representable (|v| ≤ 2^53 − 1).
pub fn number_is_safe_integer(args: &[Value]) -> Value {
    Value::Bool(matches!(args.first(),
        Some(Value::Number(n)) if n.is_finite() && n.fract() == 0.0 && n.abs() <= 9_007_199_254_740_991.0))
}

/// `Number.isNaN(v)` — true iff `v` IS the number NaN. No coercion (unlike the global `isNaN`).
pub fn number_is_nan(args: &[Value]) -> Value {
    Value::Bool(matches!(args.first(), Some(Value::Number(n)) if n.is_nan()))
}

/// `Number.isFinite(v)` — true iff `v` is a finite Number. No coercion (unlike the global `isFinite`).
pub fn number_is_finite(args: &[Value]) -> Value {
    Value::Bool(matches!(args.first(), Some(Value::Number(n)) if n.is_finite()))
}

/// Parse a string as JS `Number` does: trimmed; `""` → 0; `0x`/`0o`/`0b` radix prefixes;
/// `Infinity`/`-Infinity`; plain decimal/float; anything else → NaN. Public so the
/// tree-walk interpreter (distinct `Value` type) shares the exact coercion.
pub fn parse_numeric_string(s: &str) -> f64 {
    let t = s.trim();
    if t.is_empty() {
        return 0.0;
    }
    let radix = |rest: &str, r: u32| i64::from_str_radix(rest, r).map(|x| x as f64).unwrap_or(f64::NAN);
    if let Some(rest) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        return radix(rest, 16);
    }
    if let Some(rest) = t.strip_prefix("0o").or_else(|| t.strip_prefix("0O")) {
        return radix(rest, 8);
    }
    if let Some(rest) = t.strip_prefix("0b").or_else(|| t.strip_prefix("0B")) {
        return radix(rest, 2);
    }
    match t {
        "Infinity" | "+Infinity" => f64::INFINITY,
        "-Infinity" => f64::NEG_INFINITY,
        _ => t.parse::<f64>().unwrap_or(f64::NAN),
    }
}

/// String.fromCharCode(...codes)
pub fn string_from_char_code(args: &[Value]) -> Value {
    let s: String = args
        .iter()
        .filter_map(|v| match v {
            Value::Number(n) => char::from_u32(*n as u32),
            _ => None,
        })
        .collect();
    Value::String(s.into())
}

/// Object.keys(obj)
pub fn object_keys(args: &[Value]) -> Value {
    match args.first() {
        Some(Value::Object(obj)) => {
            let obj_borrow = obj.borrow();
            let keys: Vec<Value> = obj_borrow
                .strings
                .keys()
                .map(|k| Value::String(tishlang_core::ArcStr::from(k.as_ref())))
                .collect();
            Value::Array(VmRef::new(keys))
        }
        // `Object.keys(array)` yields the index strings "0".."len-1" (an array's own enumerable keys
        // are its indices, per JS). Previously returned `[]` — a node divergence, and the reason
        // `for (k in array)` enumerated nothing on the vm/native backends (they lower for-in to
        // for-of over `Object.keys`).
        Some(Value::Array(arr)) => {
            let len = arr.borrow().len();
            let keys: Vec<Value> = (0..len)
                .map(|i| Value::String(tishlang_core::ArcStr::from(i.to_string().as_str())))
                .collect();
            Value::Array(VmRef::new(keys))
        }
        _ => Value::Array(VmRef::new(Vec::new())),
    }
}

/// Object.values(obj)
pub fn object_values(args: &[Value]) -> Value {
    if let Some(Value::Object(obj)) = args.first() {
        let obj_borrow = obj.borrow();
        let values: Vec<Value> = obj_borrow.strings.values().cloned().collect();
        Value::Array(VmRef::new(values))
    } else {
        Value::Array(VmRef::new(Vec::new()))
    }
}

/// Object.entries(obj)
pub fn object_entries(args: &[Value]) -> Value {
    if let Some(Value::Object(obj)) = args.first() {
        let obj_borrow = obj.borrow();
        let entries: Vec<Value> = obj_borrow
            .strings
            .iter()
            .map(|(k, v)| Value::Array(VmRef::new(vec![Value::String(tishlang_core::ArcStr::from(k.as_ref())), v.clone()])))
            .collect();
        Value::Array(VmRef::new(entries))
    } else {
        Value::Array(VmRef::new(Vec::new()))
    }
}

/// Object.assign(target, ...sources)
pub fn object_assign(args: &[Value]) -> Value {
    let target = match args.first() {
        Some(Value::Object(obj)) => obj,
        _ => return Value::Null,
    };

    let additional_capacity: usize = args
        .iter()
        .skip(1)
        .map(|source| {
            if let Value::Object(src) = source {
                src.borrow().len_entries()
            } else {
                0
            }
        })
        .sum();

    let mut target_mut = target.borrow_mut();
    target_mut.strings.reserve(additional_capacity);

    for source in args.iter().skip(1) {
        if let Value::Object(src) = source {
            let src_borrow = src.borrow();
            for (k, v) in src_borrow.strings.iter() {
                target_mut.strings.insert(Arc::clone(k), v.clone());
            }
            if let Some(ss) = &src_borrow.symbols {
                if target_mut.symbols.is_none() {
                    target_mut.symbols = Some(Default::default());
                }
                let dst = target_mut.symbols.as_mut().unwrap();
                dst.extend(ss.iter().map(|(id, v)| (*id, v.clone())));
            }
        }
    }
    drop(target_mut);
    Value::Object(target.clone())
}

/// parseInt(string, radix?)
pub fn parse_int(args: &[Value]) -> Value {
    let s = args
        .first()
        .map(Value::to_display_string)
        .unwrap_or_default();
    let radix = args.get(1).and_then(|v| match v {
        Value::Number(n) => Some(*n as i32),
        _ => None,
    });
    Value::Number(js_parse_int(&s, radix))
}

/// JS `parseInt(string, radix)` semantics — shared so the tree-walk interpreter (distinct `Value`)
/// matches the vm/native exactly (#247). Skips leading whitespace + an optional sign, then strips a
/// leading `0x`/`0X` when radix is 16 (or omitted and the string is hex-prefixed); an omitted radix
/// otherwise defaults to 10. Reads the longest valid digit prefix (`parseInt("12px") === 12`),
/// accumulating in f64 to avoid the old i64-overflow `NaN`. `parseInt("0x1F", 16) === 31`.
pub fn js_parse_int(input: &str, radix_arg: Option<i32>) -> f64 {
    let s = input.trim_start();
    let (neg, rest) = if let Some(r) = s.strip_prefix('-') {
        (true, r)
    } else if let Some(r) = s.strip_prefix('+') {
        (false, r)
    } else {
        (false, s)
    };
    let mut radix = radix_arg.unwrap_or(0);
    let mut digits = rest;
    if radix == 16 || radix == 0 {
        if let Some(r) = rest.strip_prefix("0x").or_else(|| rest.strip_prefix("0X")) {
            radix = 16;
            digits = r;
        }
    }
    if radix == 0 {
        radix = 10;
    }
    if !(2..=36).contains(&radix) {
        return f64::NAN;
    }
    let mut acc = 0.0_f64;
    let mut any = false;
    for c in digits.chars() {
        match c.to_digit(radix as u32) {
            Some(d) => {
                acc = acc * radix as f64 + d as f64;
                any = true;
            }
            None => break,
        }
    }
    if !any {
        f64::NAN
    } else if neg {
        -acc
    } else {
        acc
    }
}

/// parseFloat(string)
pub fn parse_float(args: &[Value]) -> Value {
    let s = args
        .first()
        .map(Value::to_display_string)
        .unwrap_or_default();
    Value::Number(js_parse_float(&s))
}

/// JS `parseFloat`: skips leading whitespace, then parses the **longest leading prefix**
/// that's a valid float (so `parseFloat("3.14abc")` → 3.14, `parseFloat("12.3.4")` → 12.3).
/// Handles `Infinity`/`-Infinity`; returns NaN when no numeric prefix is present. Issue #36.
/// Public so the tree-walk interpreter (distinct `Value`) shares the exact behavior.
pub fn js_parse_float(s: &str) -> f64 {
    let t = s.trim_start();
    if t.starts_with("Infinity") || t.starts_with("+Infinity") {
        return f64::INFINITY;
    }
    if t.starts_with("-Infinity") {
        return f64::NEG_INFINITY;
    }
    // Take a generous run of float-shaped chars, then shrink from the right until it parses.
    let mut end = 0;
    for (i, c) in t.char_indices() {
        if c.is_ascii_digit() || matches!(c, '.' | '+' | '-' | 'e' | 'E') {
            end = i + c.len_utf8();
        } else {
            break;
        }
    }
    let mut slice = &t[..end];
    while !slice.is_empty() {
        if let Ok(n) = slice.parse::<f64>() {
            return n;
        }
        slice = &slice[..slice.len() - 1];
    }
    f64::NAN
}

/// Object.fromEntries(entries)
pub fn object_from_entries(args: &[Value]) -> Value {
    if let Some(Value::Array(entries)) = args.first() {
        let entries_borrow = entries.borrow();
        let mut obj: ObjectMap = ObjectMap::with_capacity(entries_borrow.len());

        for entry in entries_borrow.iter() {
            if let Value::Array(pair) = entry {
                let pair_borrow = pair.borrow();
                if pair_borrow.len() >= 2 {
                    let key: Arc<str> = match &pair_borrow[0] {
                        Value::String(s) => Arc::from(s.as_str()),
                        v => v.to_display_string().into(),
                    };
                    obj.insert(key, pair_borrow[1].clone());
                }
            }
        }

        Value::object(obj)
    } else {
        Value::empty_object()
    }
}
