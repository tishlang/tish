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
pub fn json_stringify(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => {
            if n.is_nan() || n.is_infinite() {
                "null".to_string()
            } else {
                n.to_string()
            }
        }
        Value::String(s) => format!("\"{}\"", escape_json_string(s)),
        Value::Array(arr) => {
            let borrowed = arr.borrow();
            let mut result = String::with_capacity(borrowed.len() * 8 + 2);
            result.push('[');
            let mut first = true;
            for item in borrowed.iter() {
                if !first {
                    result.push(',');
                }
                first = false;
                result.push_str(&json_stringify(item));
            }
            result.push(']');
            result
        }
        Value::Object(obj) => {
            let borrowed = obj.borrow();
            let mut keys: Vec<_> = borrowed.keys().collect();
            keys.sort();
            let mut result = String::with_capacity(borrowed.len() * 16 + 2);
            result.push('{');
            let mut first = true;
            for key in keys {
                if !first {
                    result.push(',');
                }
                first = false;
                result.push('"');
                result.push_str(&escape_json_string(key));
                result.push_str("\":");
                result.push_str(&json_stringify(borrowed.get(key).unwrap()));
            }
            result.push('}');
            result
        }
        Value::Function(_) => "null".to_string(),
        Value::Promise(_) => "null".to_string(),
        Value::Opaque(_) => "null".to_string(),
        #[cfg(feature = "regex")]
        Value::RegExp(_) => "null".to_string(),
    }
}

fn escape_json_string(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => result.push_str("\\\""),
            '\\' => result.push_str("\\\\"),
            '\n' => result.push_str("\\n"),
            '\r' => result.push_str("\\r"),
            '\t' => result.push_str("\\t"),
            c if c.is_control() => result.push_str(&format!("\\u{:04x}", c as u32)),
            c => result.push(c),
        }
    }
    result
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
