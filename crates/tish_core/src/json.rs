//! JSON parsing and stringification for Tish values.

use crate::{Value, VmRef};
use std::sync::Arc;

/// Per-`json_parse`-call cache of object-key text → shared `Arc<str>`. A JSON array of records
/// repeats the same handful of keys across thousands of objects; interning allocates each key
/// once and `Arc::clone`s it thereafter, instead of a fresh `Arc<str>` allocation per occurrence.
type KeyCache = ahash::AHashMap<Box<str>, Arc<str>>;

#[inline]
fn intern_key(cache: &mut KeyCache, s: &str) -> Arc<str> {
    if let Some(existing) = cache.get(s) {
        return Arc::clone(existing);
    }
    let arc: Arc<str> = Arc::from(s);
    cache.insert(Box::from(s), Arc::clone(&arc));
    arc
}

/// Append `n` to `buf` exactly as JS `JSON.stringify` formats a number. Integer-valued finite
/// numbers within the safe-integer range (`|n| < 2^53`) take a fast `i64` path — bit-identical to
/// JS for every such value (verified against Node over 100k values) and far cheaper than the `f64`
/// formatter. Everything else uses the ECMAScript `Number::toString` (`js_number_to_string`),
/// which matches JS where Rust's `{}` Display would not (e.g. `1e21`, `1e-7`). NaN/∞ → `null`.
/// Public so the interpreter's separate JSON path formats numbers identically (single source of
/// truth → interp/vm/native/node agree).
#[inline]
pub fn write_json_number(buf: &mut String, n: f64) {
    if n.is_nan() || n.is_infinite() {
        buf.push_str("null");
        return;
    }
    if n.fract() == 0.0 && n.abs() < 9_007_199_254_740_992.0 {
        let mut b = itoa::Buffer::new();
        buf.push_str(b.format(n as i64));
        return;
    }
    crate::js_number_to_string_into(buf, n);
}

/// Scan a string body (the input with its opening `"` already removed) for the closing quote.
/// Returns `Ok(Some((body, rest)))` when the string is escape-free — `body` is the raw contents and
/// `rest` is the input just past the closing quote, so the value is built in a single allocation
/// with no per-char decode. Returns `Ok(None)` on the first backslash (caller uses the escape
/// decoder) and `Err` if unterminated. Only stops on the ASCII bytes `"`/`\\`, so the byte index
/// always lands on a UTF-8 char boundary — multi-byte sequences pass through inside `body` intact.
#[inline]
fn scan_escape_free(body: &str) -> Result<Option<(&str, &str)>, String> {
    let bytes = body.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => return Ok(None),
            b'"' => return Ok(Some((&body[..i], &body[i + 1..]))),
            _ => i += 1,
        }
    }
    Err("Unterminated string".to_string())
}

/// Parse JSON string into a Value.
pub fn json_parse(json: &str) -> Result<Value, String> {
    let json = json.trim();
    if json.is_empty() {
        return Err("SyntaxError: Unexpected end of JSON input".to_string());
    }
    let mut cache = KeyCache::default();
    let (value, rest) = parse_value(json, 0, &mut cache)?;
    if !rest.trim().is_empty() {
        return Err("SyntaxError: Unexpected token at end of JSON".to_string());
    }
    Ok(value)
}

/// Stringify a Value to JSON.
///
/// Single-buffer write strategy: all nested values append into one
/// `String` via [`json_stringify_into`], so we never allocate a transient
/// per-node `String` only to copy + drop it on the way back up. For a
/// 20-row TFB `/queries` response (~40 numbers, 2 keys × 20 = ~80 string
/// ops) that saves dozens of small allocations per request.
fn json_stringify_capacity_hint(value: &Value) -> usize {
    match value {
        Value::Array(arr) => {
            let n = arr.borrow().len();
            if n > 64 {
                // json_roundtrip / large API payloads: ~80–100 B per row is typical.
                n.saturating_mul(96).max(256)
            } else {
                256
            }
        }
        Value::Object(obj) => {
            let n = obj.borrow().strings.len();
            if n > 32 {
                n.saturating_mul(128).max(256)
            } else {
                256
            }
        }
        _ => 256,
    }
}

pub fn json_stringify(value: &Value) -> String {
    let mut buf = String::with_capacity(json_stringify_capacity_hint(value));
    json_stringify_into(&mut buf, value);
    buf
}

/// Append a JSON-stringified `value` to `buf`. Used by JSON.stringify for
/// the recursive case so we don't pay for an intermediate `String` per
/// node.
///
/// Cyclic object graphs (`a.self = a`) are detected and — matching JS, which throws
/// `TypeError: Converting circular structure to JSON` — set a pending throw and emit `null` for the
/// back-reference instead of recursing forever (#381: this was an unrecoverable thread hang, directly
/// reachable from HTTP response serialization).
pub fn json_stringify_into(buf: &mut String, value: &Value) {
    // Track the CURRENT ancestor path only (not all-visited): a node repeated across sibling branches
    // is a legal DAG and must serialize twice; only a back-edge to an ancestor is a cycle.
    let mut ancestors: Vec<*const ()> = Vec::new();
    json_stringify_into_guarded(buf, value, &mut ancestors);
}

/// Set the JS "circular structure" TypeError as the pending throw (once) — mirrors the
/// `{ error: <msg> }` shape `tish_builtins::helpers::make_error_value` produces (that crate sits above
/// this one, so the shape is reproduced here rather than imported).
fn signal_circular_json_throw() {
    if !crate::has_pending_throw() {
        crate::set_pending_throw(Value::object_from_pairs([(
            std::sync::Arc::from("error"),
            Value::String("TypeError: Converting circular structure to JSON".into()),
        )]));
    }
}

fn json_stringify_into_guarded(buf: &mut String, value: &Value, ancestors: &mut Vec<*const ()>) {
    match value {
        Value::Null => buf.push_str("null"),
        Value::Bool(true) => buf.push_str("true"),
        Value::Bool(false) => buf.push_str("false"),
        Value::Number(n) => write_json_number(buf, *n),
        Value::String(s) => {
            buf.push('"');
            escape_json_string_into(buf, s);
            buf.push('"');
        }
        Value::Array(arr) => {
            let ptr = arr.as_ptr();
            if ancestors.contains(&ptr) {
                signal_circular_json_throw();
                buf.push_str("null");
                return;
            }
            ancestors.push(ptr);
            let borrowed = arr.borrow();
            buf.push('[');
            for (i, item) in borrowed.iter().enumerate() {
                if i > 0 {
                    buf.push(',');
                }
                json_stringify_into_guarded(buf, item, ancestors);
            }
            buf.push(']');
            drop(borrowed);
            ancestors.pop();
        }
        Value::NumberArray(arr) => {
            let borrowed = arr.borrow();
            buf.push('[');
            for (i, n) in borrowed.iter().enumerate() {
                if i > 0 {
                    buf.push(',');
                }
                write_json_number(buf, *n);
            }
            buf.push(']');
        }
        Value::Object(obj) => {
            let ptr = obj.as_ptr();
            if ancestors.contains(&ptr) {
                signal_circular_json_throw();
                buf.push_str("null");
                return;
            }
            ancestors.push(ptr);
            let borrowed = obj.borrow();
            // Iterate in insertion order (PropMap preserves it) — matches JS/Node
            // and `Object.keys`. No intermediate key Vec, no sort: one fewer
            // allocation per object on the JSON hot path (e.g. TFB /json, /db).
            buf.push('{');
            for (i, (key, val)) in borrowed.strings.iter().enumerate() {
                if i > 0 {
                    buf.push(',');
                }
                buf.push('"');
                escape_json_string_into(buf, key);
                buf.push_str("\":");
                json_stringify_into_guarded(buf, val, ancestors);
            }
            buf.push('}');
            drop(borrowed);
            ancestors.pop();
        }
        Value::Function(_) | Value::Promise(_) | Value::Opaque(_) | Value::Symbol(_) => {
            buf.push_str("null");
        }
        #[cfg(feature = "regex")]
        Value::RegExp(_) => buf.push_str("null"),
    }
}

/// Append an escaped JSON string body (without the surrounding quotes)
/// to `buf`. Optimised for the common case where the input is ASCII and
/// contains no characters that need escaping — we fast-pass the bytes
/// straight through, only falling into the per-char path on a hit.
fn escape_json_string_into(buf: &mut String, s: &str) {
    let bytes = s.as_bytes();
    let mut start = 0usize;
    for (i, &b) in bytes.iter().enumerate() {
        // Anything < 0x20 is a JSON control char that must be escaped;
        // 0x22 (`"`) and 0x5C (`\`) also need an explicit escape; bytes
        // ≥ 0x80 are the start of a multi-byte UTF-8 sequence, which is
        // valid JSON as-is.
        if b < 0x20 || b == b'"' || b == b'\\' {
            // Flush the run of clean bytes before this one in one push.
            if start < i {
                // SAFETY: `s` is `&str`, every byte in `start..i` was a
                // single-byte ASCII char (we only stop on ASCII triggers
                // below 0x80), so the slice is a valid `&str`.
                buf.push_str(&s[start..i]);
            }
            match b {
                b'"' => buf.push_str("\\\""),
                b'\\' => buf.push_str("\\\\"),
                b'\n' => buf.push_str("\\n"),
                b'\r' => buf.push_str("\\r"),
                b'\t' => buf.push_str("\\t"),
                b'\x08' => buf.push_str("\\b"),
                b'\x0c' => buf.push_str("\\f"),
                _ => {
                    use std::fmt::Write;
                    let _ = write!(buf, "\\u{:04x}", b as u32);
                }
            }
            start = i + 1;
        }
    }
    if start < bytes.len() {
        buf.push_str(&s[start..]);
    }
}

/// Max nesting depth for `JSON.parse`. Bounds recursion so deeply-nested untrusted
/// input errors instead of overflowing the stack — a Rust stack overflow aborts the
/// whole process (uncatchable, SIGABRT). 128 matches serde_json's default limit.
const MAX_JSON_DEPTH: usize = 128;

fn parse_value<'a>(
    input: &'a str,
    depth: usize,
    cache: &mut KeyCache,
) -> Result<(Value, &'a str), String> {
    let input = input.trim_start();
    if input.is_empty() {
        return Err("Unexpected end of JSON input".to_string());
    }

    match input.chars().next().unwrap() {
        'n' => parse_null(input),
        't' | 'f' => parse_bool(input),
        '"' => parse_string(input),
        '[' => parse_array(input, depth, cache),
        '{' => parse_object(input, depth, cache),
        c if c == '-' || c.is_ascii_digit() => parse_number(input),
        c => Err(format!("Unexpected character '{}' in JSON", c)),
    }
}

fn parse_null(input: &str) -> Result<(Value, &str), String> {
    if let Some(rest) = input.strip_prefix("null") {
        Ok((Value::Null, rest))
    } else {
        Err("Expected 'null'".to_string())
    }
}

fn parse_bool(input: &str) -> Result<(Value, &str), String> {
    if let Some(rest) = input.strip_prefix("true") {
        Ok((Value::Bool(true), rest))
    } else if let Some(rest) = input.strip_prefix("false") {
        Ok((Value::Bool(false), rest))
    } else {
        Err("Expected 'true' or 'false'".to_string())
    }
}

fn parse_string(input: &str) -> Result<(Value, &str), String> {
    // Fast path: an escape-free string is a direct slice of the input — one allocation, no decode.
    let body = &input[1..]; // skip opening quote (ASCII, safe byte index)
    if let Some((s, rest)) = scan_escape_free(body)? {
        return Ok((Value::String(s.into()), rest));
    }
    parse_string_escaped(input)
}

/// Read an object key, interning it through `cache` so repeated keys share one `Arc<str>` instead
/// of allocating a fresh one per occurrence. Escape-free keys (the common case) look up the cache
/// with the borrowed slice — zero allocations on a hit.
fn parse_key<'a>(input: &'a str, cache: &mut KeyCache) -> Result<(Arc<str>, &'a str), String> {
    let body = &input[1..];
    if let Some((s, rest)) = scan_escape_free(body)? {
        return Ok((intern_key(cache, s), rest));
    }
    let (val, rest) = parse_string_escaped(input)?;
    match val {
        Value::String(s) => Ok((intern_key(cache, s.as_str()), rest)),
        _ => unreachable!("parse_string_escaped always yields Value::String"),
    }
}

/// Decode a JSON string that contains at least one escape (the slow path).
fn parse_string_escaped(input: &str) -> Result<(Value, &str), String> {
    let input = &input[1..]; // skip opening quote
    let mut result = String::new();
    let mut chars = input.chars().peekable();
    let mut byte_count = 0;

    loop {
        match chars.next() {
            None => return Err("Unterminated string".to_string()),
            Some('"') => {
                byte_count += 1;
                break;
            }
            Some('\\') => {
                byte_count += 1;
                match chars.next() {
                    Some('n') => {
                        result.push('\n');
                        byte_count += 1;
                    }
                    Some('r') => {
                        result.push('\r');
                        byte_count += 1;
                    }
                    Some('t') => {
                        result.push('\t');
                        byte_count += 1;
                    }
                    Some('\\') => {
                        result.push('\\');
                        byte_count += 1;
                    }
                    Some('"') => {
                        result.push('"');
                        byte_count += 1;
                    }
                    Some('/') => {
                        result.push('/');
                        byte_count += 1;
                    }
                    Some('u') => {
                        byte_count += 1;
                        let mut hex = String::new();
                        for _ in 0..4 {
                            if let Some(c) = chars.next() {
                                hex.push(c);
                                byte_count += c.len_utf8();
                            }
                        }
                        if let Ok(n) = u32::from_str_radix(&hex, 16) {
                            if let Some(c) = char::from_u32(n) {
                                result.push(c);
                            }
                        }
                    }
                    Some(c) => {
                        result.push(c);
                        byte_count += c.len_utf8();
                    }
                    None => return Err("Unterminated escape sequence".to_string()),
                }
            }
            Some(c) => {
                result.push(c);
                byte_count += c.len_utf8();
            }
        }
    }

    Ok((Value::String(result.into()), &input[byte_count..]))
}

fn parse_number(input: &str) -> Result<(Value, &str), String> {
    // Byte scan (all number chars are ASCII) — O(token), not O(remaining input).
    // The old `input.chars().collect::<Vec<char>>()` per number made parsing an
    // N-number array O(N^2): a CPU-exhaustion DoS on untrusted JSON.
    let bytes = input.as_bytes();
    let mut end = 0;

    let neg = bytes.first() == Some(&b'-');
    if neg {
        end += 1;
    }
    let int_start = end;
    while end < bytes.len() && bytes[end].is_ascii_digit() {
        end += 1;
    }
    let int_len = end - int_start;
    let mut is_integer = true;
    if bytes.get(end) == Some(&b'.') {
        is_integer = false;
        end += 1;
        while end < bytes.len() && bytes[end].is_ascii_digit() {
            end += 1;
        }
    }
    if matches!(bytes.get(end), Some(&b'e') | Some(&b'E')) {
        is_integer = false;
        end += 1;
        if matches!(bytes.get(end), Some(&b'+') | Some(&b'-')) {
            end += 1;
        }
        while end < bytes.len() && bytes[end].is_ascii_digit() {
            end += 1;
        }
    }

    // `end` lands on an ASCII boundary, so slicing `input` by byte index is valid.
    let num_str = &input[..end];
    // Integer fast-path: a pure integer of ≤15 digits fits `i64` and is exact in `f64`
    // (|value| < 10^15 < 2^53), so `i64 as f64` equals the full float parse but is far cheaper —
    // and JSON arrays of records are integer-heavy. Negative zero is excluded: `i64` loses the
    // sign, but `JSON.parse("-0")` must yield `-0.0` (matches std `f64` parse and Node).
    if is_integer && (1..=15).contains(&int_len) {
        if let Ok(i) = num_str.parse::<i64>() {
            let n = if neg && i == 0 { -0.0 } else { i as f64 };
            return Ok((Value::Number(n), &input[end..]));
        }
    }
    num_str
        .parse::<f64>()
        .map(|n| (Value::Number(n), &input[end..]))
        .map_err(|_| format!("Invalid number: {}", num_str))
}

fn parse_array<'a>(
    input: &'a str,
    depth: usize,
    cache: &mut KeyCache,
) -> Result<(Value, &'a str), String> {
    if depth >= MAX_JSON_DEPTH {
        return Err("JSON nesting too deep".to_string());
    }
    let mut input = &input[1..]; // skip '['
    let mut items = Vec::new();

    input = input.trim_start();
    if let Some(rest) = input.strip_prefix(']') {
        return Ok((Value::Array(VmRef::new(items)), rest));
    }

    loop {
        let (value, rest) = parse_value(input, depth + 1, cache)?;
        items.push(value);
        input = rest.trim_start();

        match input.chars().next() {
            Some(',') => input = &input[1..],
            Some(']') => return Ok((Value::Array(VmRef::new(items)), &input[1..])),
            _ => return Err("Expected ',' or ']' in array".to_string()),
        }
    }
}

fn parse_object<'a>(
    input: &'a str,
    depth: usize,
    cache: &mut KeyCache,
) -> Result<(Value, &'a str), String> {
    if depth >= MAX_JSON_DEPTH {
        return Err("JSON nesting too deep".to_string());
    }
    let mut input = &input[1..]; // skip '{'
    // Build the insertion-ordered `PropMap` directly. The old path collected into an `AHashMap`
    // and then re-inserted every pair into a `PropMap` via `from_strings` — a wasted map + rehash
    // per object, AND the `AHashMap` iteration order scrambled JSON key order (its `RandomState`
    // reseeds per process), so `Object.keys` after `JSON.parse` came out in a non-spec order.
    let mut map = crate::PropMap::new();

    input = input.trim_start();
    if let Some(rest) = input.strip_prefix('}') {
        return Ok((
            Value::Object(VmRef::new(crate::ObjectData {
                strings: map,
                symbols: None,
            })),
            rest,
        ));
    }

    loop {
        input = input.trim_start();
        if !input.starts_with('"') {
            return Err("Expected string key in object".to_string());
        }

        let (key, rest) = parse_key(input, cache)?;

        input = rest.trim_start();
        if !input.starts_with(':') {
            return Err("Expected ':' after key in object".to_string());
        }
        input = &input[1..];

        let (value, rest) = parse_value(input, depth + 1, cache)?;
        map.insert(key, value);
        input = rest.trim_start();

        match input.chars().next() {
            Some(',') => input = &input[1..],
            Some('}') => {
                return Ok((
                    Value::Object(VmRef::new(crate::ObjectData {
                        strings: map,
                        symbols: None,
                    })),
                    &input[1..],
                ));
            }
            _ => return Err("Expected ',' or '}' in object".to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// #381: a self-referential array must NOT loop forever — `JSON.stringify` terminates, emits a
    /// finite string, and (matching JS) leaves a pending TypeError. The test *completing* is itself
    /// the assertion that the former infinite hang is gone.
    #[test]
    fn json_stringify_cyclic_array_terminates_and_throws() {
        let _ = crate::take_pending_throw(); // isolate from any prior thread-local throw
        let a = Value::Array(crate::VmRef::new(Vec::new()));
        if let Value::Array(inner) = &a {
            inner.borrow_mut().push(a.clone()); // a = [a]
        }
        let s = json_stringify(&a);
        assert_eq!(s, "[null]", "back-reference serializes as null, finite output");
        assert!(crate::has_pending_throw(), "cyclic stringify must set a pending TypeError");
        let _ = crate::take_pending_throw(); // clean up thread-local for other tests
    }

    /// A shared (non-cyclic) node repeated across sibling branches is a legal DAG and must serialize
    /// twice — the ancestor-only tracking must NOT misflag it as a cycle.
    #[test]
    fn json_stringify_shared_dag_node_is_not_a_cycle() {
        let _ = crate::take_pending_throw();
        let shared = Value::Array(crate::VmRef::new(vec![Value::Number(1.0)]));
        let root = Value::Array(crate::VmRef::new(vec![shared.clone(), shared.clone()]));
        let s = json_stringify(&root);
        assert_eq!(s, "[[1],[1]]", "a shared node is a DAG, not a cycle");
        assert!(!crate::has_pending_throw(), "a DAG must NOT throw");
    }

    #[test]
    fn test_parse_primitives() {
        assert!(matches!(json_parse("null").unwrap(), Value::Null));
        assert!(matches!(json_parse("true").unwrap(), Value::Bool(true)));
        assert!(matches!(json_parse("false").unwrap(), Value::Bool(false)));
        assert!(matches!(json_parse("42").unwrap(), Value::Number(n) if n == 42.0));
        assert!(
            matches!(json_parse("\"hello\"").unwrap(), Value::String(s) if s.as_str() == "hello")
        );
    }

    #[test]
    fn test_roundtrip() {
        let original = "{\"name\":\"test\",\"count\":42}";
        let value = json_parse(original).unwrap();
        let stringified = json_stringify(&value);
        let reparsed = json_parse(&stringified).unwrap();

        match (&value, &reparsed) {
            (Value::Object(a), Value::Object(b)) => {
                assert_eq!(a.borrow().len_entries(), b.borrow().len_entries());
            }
            _ => panic!("Expected objects"),
        }
    }

    #[test]
    fn deeply_nested_json_is_rejected_not_crash() {
        // C1 regression: deeply-nested untrusted input must error at the depth limit,
        // never recurse deep enough to overflow the stack (an uncatchable SIGABRT that
        // would crash the whole process / HTTP worker).
        let under = format!("{}{}", "[".repeat(100), "]".repeat(100));
        assert!(json_parse(&under).is_ok(), "100 < limit should parse");
        let over = format!("{}{}", "[".repeat(200), "]".repeat(200));
        assert!(json_parse(&over).is_err(), "200 > limit must error");
        // Pathological depth must still just error (fast), not overflow the stack.
        let huge = format!("{}{}", "[".repeat(200_000), "]".repeat(200_000));
        assert!(json_parse(&huge).is_err(), "huge depth must error, not crash");
    }

    #[test]
    fn large_number_array_parses_correctly() {
        // C2 regression: parse_number byte-scans (O(token)); the old chars().collect()
        // over the whole remaining input made an N-number array O(N^2) — a CPU DoS.
        let n = 50_000;
        let body = format!("[{}]", vec!["7"; n].join(","));
        match json_parse(&body).unwrap() {
            Value::Array(arr) => assert_eq!(arr.borrow().len(), n),
            _ => panic!("expected array"),
        }
    }
}
