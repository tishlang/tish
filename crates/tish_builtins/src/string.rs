//! String builtin methods.
//!
//! All indices use character (Unicode scalar) positions for consistency with
//! JavaScript, matching .length and .charAt(). Byte offsets are never exposed.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use tishlang_core::Value;
use crate::helpers::normalize_index;

/// Byte offset -> character index.
fn byte_to_char_index(s: &str, byte_offset: usize) -> usize {
    s.char_indices()
        .take_while(|(i, _)| *i < byte_offset)
        .count()
}

/// Character index -> byte offset.
fn char_to_byte_offset(s: &str, char_index: usize) -> usize {
    s.char_indices()
        .nth(char_index)
        .map(|(i, _)| i)
        .unwrap_or(s.len())
}

/// Create a new string Value from a string slice.
pub fn from_str(s: &str) -> Value {
    Value::String(Arc::from(s))
}

/// Get the length of a string (character count).
pub fn len(s: &Value) -> Option<usize> {
    match s {
        Value::String(str) => Some(str.chars().count()),
        _ => None,
    }
}

/// Returns character index of first occurrence, or -1. Optional fromIndex (JS indexOf).
pub fn index_of(s: &Value, search: &Value, from: Option<&Value>) -> Value {
    if let (Value::String(s), Value::String(search)) = (s, search) {
        let from_char = match from {
            Some(Value::Number(n)) if *n >= 0.0 => (*n as usize).min(s.chars().count()),
            _ => 0,
        };
        let byte_start = char_to_byte_offset(s, from_char);
        let search_str = search.as_ref();
        if let Some(byte_pos) = s[byte_start..].find(search_str) {
            let char_idx = from_char + byte_to_char_index(&s[byte_start..], byte_pos);
            Value::Number(char_idx as f64)
        } else {
            Value::Number(-1.0)
        }
    } else {
        Value::Number(-1.0)
    }
}

pub fn includes(s: &Value, search: &Value, from: Option<&Value>) -> Value {
    if let (Value::String(s), Value::String(search)) = (s, search) {
        let from_char = match from {
            Some(Value::Number(n)) if *n >= 0.0 => (*n as usize).min(s.chars().count()),
            Some(Value::Number(n)) if *n < 0.0 => {
                let len = s.chars().count() as i64;
                ((len + *n as i64).max(0)) as usize
            }
            _ => 0,
        };
        let byte_start = char_to_byte_offset(s, from_char);
        Value::Bool(s[byte_start..].contains(search.as_ref()))
    } else {
        Value::Bool(false)
    }
}

pub fn slice(s: &Value, start: &Value, end: &Value) -> Value {
    if let Value::String(s) = s {
        let chars: Vec<char> = s.chars().collect();
        let len = chars.len() as i64;
        let (si, ei) = (normalize_index(start, len, 0), normalize_index(end, len, len as usize));
        let result: String = if si < ei {
            chars[si..ei].iter().collect()
        } else {
            String::new()
        };
        Value::String(result.into())
    } else {
        Value::Null
    }
}

pub fn substring(s: &Value, start: &Value, end: &Value) -> Value {
    fn bounds(start: &Value, end: &Value, len: usize) -> (usize, usize) {
        let si = match start {
            Value::Number(n) => (*n as usize).min(len),
            _ => 0,
        };
        let ei = match end {
            Value::Null => len,
            Value::Number(n) => (*n as usize).min(len),
            _ => len,
        };
        (si.min(ei), si.max(ei))
    }
    if let Value::String(s) = s {
        let chars: Vec<char> = s.chars().collect();
        let (ss, ee) = bounds(start, end, chars.len());
        let result: String = chars[ss..ee].iter().collect();
        Value::String(result.into())
    } else {
        Value::Null
    }
}

pub fn split(s: &Value, sep: &Value) -> Value {
    if let Value::String(s) = s {
        let separator = match sep {
            Value::String(ss) => ss.as_ref(),
            _ => return Value::Array(Rc::new(RefCell::new(vec![Value::String(Arc::clone(s))]))),
        };
        let parts: Vec<Value> = s.split(separator)
            .map(|p| Value::String(p.into()))
            .collect();
        Value::Array(Rc::new(RefCell::new(parts)))
    } else {
        Value::Null
    }
}

pub fn trim(s: &Value) -> Value {
    if let Value::String(s) = s {
        Value::String(s.trim().into())
    } else {
        Value::Null
    }
}

pub fn to_upper_case(s: &Value) -> Value {
    if let Value::String(s) = s {
        Value::String(s.to_uppercase().into())
    } else {
        Value::Null
    }
}

pub fn to_lower_case(s: &Value) -> Value {
    if let Value::String(s) = s {
        Value::String(s.to_lowercase().into())
    } else {
        Value::Null
    }
}

pub fn starts_with(s: &Value, search: &Value) -> Value {
    if let (Value::String(s), Value::String(search)) = (s, search) {
        Value::Bool(s.starts_with(search.as_ref()))
    } else {
        Value::Bool(false)
    }
}

pub fn ends_with(s: &Value, search: &Value) -> Value {
    if let (Value::String(s), Value::String(search)) = (s, search) {
        Value::Bool(s.ends_with(search.as_ref()))
    } else {
        Value::Bool(false)
    }
}

fn replace_impl(s: &Value, search: &Value, replacement: &Value, all: bool) -> Value {
    if let Value::String(s) = s {
        let search_str = match search { Value::String(ss) => ss.as_ref(), _ => return Value::String(Arc::clone(s)) };
        let repl_str = match replacement { Value::String(ss) => ss.as_ref(), _ => "" };
        let result = if all { s.replace(search_str, repl_str) } else { s.replacen(search_str, repl_str, 1) };
        Value::String(result.into())
    } else {
        Value::Null
    }
}

pub fn replace(s: &Value, search: &Value, replacement: &Value) -> Value {
    replace_impl(s, search, replacement, false)
}

pub fn replace_all(s: &Value, search: &Value, replacement: &Value) -> Value {
    replace_impl(s, search, replacement, true)
}

fn char_at_idx(s: &str, idx: usize) -> Option<char> {
    s.chars().nth(idx)
}

pub fn char_at(s: &Value, idx: &Value) -> Value {
    if let Value::String(s) = s {
        let idx = match idx { Value::Number(n) => *n as usize, _ => 0 };
        char_at_idx(s, idx).map(|c| Value::String(c.to_string().into())).unwrap_or(Value::String("".into()))
    } else {
        Value::Null
    }
}

pub fn char_code_at(s: &Value, idx: &Value) -> Value {
    if let Value::String(s) = s {
        let idx = match idx { Value::Number(n) => *n as usize, _ => 0 };
        char_at_idx(s, idx).map(|c| Value::Number(c as u32 as f64)).unwrap_or(Value::Number(f64::NAN))
    } else {
        Value::Null
    }
}

pub fn repeat(s: &Value, count: &Value) -> Value {
    if let Value::String(s) = s {
        let count = match count {
            Value::Number(n) if *n >= 0.0 => *n as usize,
            _ => 0,
        };
        Value::String(s.repeat(count).into())
    } else {
        Value::Null
    }
}

fn pad_impl(s: &Value, target_len: &Value, pad: &Value, at_start: bool) -> Value {
    if let Value::String(s) = s {
        let target_len = match target_len {
            Value::Number(n) => *n as usize,
            _ => return Value::String(Arc::clone(s)),
        };
        let pad_str = match pad {
            Value::String(p) if !p.is_empty() => p.as_ref(),
            _ => " ",
        };
        let char_count = s.chars().count();
        if char_count >= target_len {
            return Value::String(Arc::clone(s));
        }
        let needed = target_len - char_count;
        let padding: String = pad_str.chars().cycle().take(needed).collect();
        let result = if at_start { format!("{}{}", padding, s) } else { format!("{}{}", s, padding) };
        Value::String(result.into())
    } else {
        Value::Null
    }
}

pub fn pad_start(s: &Value, target_len: &Value, pad: &Value) -> Value {
    pad_impl(s, target_len, pad, true)
}

pub fn pad_end(s: &Value, target_len: &Value, pad: &Value) -> Value {
    pad_impl(s, target_len, pad, false)
}
