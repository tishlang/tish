//! JSON parsing and stringification for Tish values.

use crate::{Value, VmRef};
use std::sync::Arc;

/// Parse JSON string into a Value.
pub fn json_parse(json: &str) -> Result<Value, String> {
    let json = json.trim();
    if json.is_empty() {
        return Err("SyntaxError: Unexpected end of JSON input".to_string());
    }
    let (value, rest) = parse_value(json)?;
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
pub fn json_stringify(value: &Value) -> String {
    // 256 B is enough for typical TFB responses (`/db` is 31 B,
    // `/queries=20` is ~700 B). Larger payloads reallocate normally.
    let mut buf = String::with_capacity(256);
    json_stringify_into(&mut buf, value);
    buf
}

/// Append a JSON-stringified `value` to `buf`. Used by JSON.stringify for
/// the recursive case so we don't pay for an intermediate `String` per
/// node.
pub fn json_stringify_into(buf: &mut String, value: &Value) {
    match value {
        Value::Null => buf.push_str("null"),
        Value::Bool(true) => buf.push_str("true"),
        Value::Bool(false) => buf.push_str("false"),
        Value::Number(n) => {
            if n.is_nan() || n.is_infinite() {
                buf.push_str("null");
            } else {
                // `write!` avoids the heap allocation that `to_string`
                // produces. The f64 → decimal formatter is the same
                // either way (`std::fmt::Display`).
                use std::fmt::Write;
                let _ = write!(buf, "{}", n);
            }
        }
        Value::String(s) => {
            buf.push('"');
            escape_json_string_into(buf, s);
            buf.push('"');
        }
        Value::Array(arr) => {
            let borrowed = arr.borrow();
            buf.push('[');
            for (i, item) in borrowed.iter().enumerate() {
                if i > 0 {
                    buf.push(',');
                }
                json_stringify_into(buf, item);
            }
            buf.push(']');
        }
        Value::Object(obj) => {
            let borrowed = obj.borrow();
            // Sort keys for deterministic output. Pre-allocate to avoid
            // a fresh `Vec` realloc inside `keys().collect()`.
            let mut keys: Vec<&Arc<str>> = Vec::with_capacity(borrowed.len());
            keys.extend(borrowed.keys());
            keys.sort_unstable_by(|a, b| a.as_ref().cmp(b.as_ref()));
            buf.push('{');
            for (i, key) in keys.into_iter().enumerate() {
                if i > 0 {
                    buf.push(',');
                }
                buf.push('"');
                escape_json_string_into(buf, key);
                buf.push_str("\":");
                json_stringify_into(buf, borrowed.get(key).unwrap());
            }
            buf.push('}');
        }
        Value::Function(_) | Value::Promise(_) | Value::Opaque(_) => buf.push_str("null"),
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
                buf.push_str(unsafe { std::str::from_utf8_unchecked(&bytes[start..i]) });
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
        buf.push_str(unsafe { std::str::from_utf8_unchecked(&bytes[start..]) });
    }
}

#[allow(dead_code)]
fn escape_json_string(s: &str) -> String {
    let mut buf = String::with_capacity(s.len());
    escape_json_string_into(&mut buf, s);
    buf
}

fn parse_value(input: &str) -> Result<(Value, &str), String> {
    let input = input.trim_start();
    if input.is_empty() {
        return Err("Unexpected end of JSON input".to_string());
    }

    match input.chars().next().unwrap() {
        'n' => parse_null(input),
        't' | 'f' => parse_bool(input),
        '"' => parse_string(input),
        '[' => parse_array(input),
        '{' => parse_object(input),
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
    let mut end = 0;
    let chars: Vec<char> = input.chars().collect();

    if chars.get(end) == Some(&'-') {
        end += 1;
    }

    while end < chars.len() && chars[end].is_ascii_digit() {
        end += 1;
    }

    if chars.get(end) == Some(&'.') {
        end += 1;
        while end < chars.len() && chars[end].is_ascii_digit() {
            end += 1;
        }
    }

    if chars.get(end) == Some(&'e') || chars.get(end) == Some(&'E') {
        end += 1;
        if chars.get(end) == Some(&'+') || chars.get(end) == Some(&'-') {
            end += 1;
        }
        while end < chars.len() && chars[end].is_ascii_digit() {
            end += 1;
        }
    }

    let num_str: String = chars[..end].iter().collect();
    let byte_len: usize = chars[..end].iter().map(|c| c.len_utf8()).sum();

    num_str
        .parse::<f64>()
        .map(|n| (Value::Number(n), &input[byte_len..]))
        .map_err(|_| format!("Invalid number: {}", num_str))
}

fn parse_array(input: &str) -> Result<(Value, &str), String> {
    let mut input = &input[1..]; // skip '['
    let mut items = Vec::new();

    input = input.trim_start();
    if let Some(rest) = input.strip_prefix(']') {
        return Ok((Value::Array(VmRef::new(items)), rest));
    }

    loop {
        let (value, rest) = parse_value(input)?;
        items.push(value);
        input = rest.trim_start();

        match input.chars().next() {
            Some(',') => input = &input[1..],
            Some(']') => return Ok((Value::Array(VmRef::new(items)), &input[1..])),
            _ => return Err("Expected ',' or ']' in array".to_string()),
        }
    }
}

fn parse_object(input: &str) -> Result<(Value, &str), String> {
    let mut input = &input[1..]; // skip '{'
    let mut map = crate::ObjectMap::default();

    input = input.trim_start();
    if let Some(rest) = input.strip_prefix('}') {
        return Ok((Value::Object(VmRef::new(map)), rest));
    }

    loop {
        input = input.trim_start();
        if !input.starts_with('"') {
            return Err("Expected string key in object".to_string());
        }

        let (key_val, rest) = parse_string(input)?;
        let key: Arc<str> = match key_val {
            Value::String(s) => s,
            _ => unreachable!(),
        };

        input = rest.trim_start();
        if !input.starts_with(':') {
            return Err("Expected ':' after key in object".to_string());
        }
        input = &input[1..];

        let (value, rest) = parse_value(input)?;
        map.insert(key, value);
        input = rest.trim_start();

        match input.chars().next() {
            Some(',') => input = &input[1..],
            Some('}') => return Ok((Value::Object(VmRef::new(map)), &input[1..])),
            _ => return Err("Expected ',' or '}' in object".to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_primitives() {
        assert!(matches!(json_parse("null").unwrap(), Value::Null));
        assert!(matches!(json_parse("true").unwrap(), Value::Bool(true)));
        assert!(matches!(json_parse("false").unwrap(), Value::Bool(false)));
        assert!(matches!(json_parse("42").unwrap(), Value::Number(n) if n == 42.0));
        assert!(
            matches!(json_parse("\"hello\"").unwrap(), Value::String(s) if s.as_ref() == "hello")
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
                assert_eq!(a.borrow().len(), b.borrow().len());
            }
            _ => panic!("Expected objects"),
        }
    }
}
