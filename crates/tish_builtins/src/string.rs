//! String builtin methods.
//!
//! Shared string method implementations used by both tish_runtime (compiled code)
//! and can be adapted for tish_eval (interpreter).

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use tish_core::Value;

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

pub fn index_of(s: &Value, search: &Value) -> Value {
    if let (Value::String(s), Value::String(search)) = (s, search) {
        Value::Number(s.find(search.as_ref()).map(|i| i as f64).unwrap_or(-1.0))
    } else {
        Value::Number(-1.0)
    }
}

pub fn includes(s: &Value, search: &Value) -> Value {
    if let (Value::String(s), Value::String(search)) = (s, search) {
        Value::Bool(s.contains(search.as_ref()))
    } else {
        Value::Bool(false)
    }
}

pub fn slice(s: &Value, start: &Value, end: &Value) -> Value {
    if let Value::String(s) = s {
        if s.is_ascii() {
            let len = s.len() as i64;
            let start_idx = match start {
                Value::Number(n) => {
                    let n = *n as i64;
                    if n < 0 { (len + n).max(0) as usize } else { n.min(len) as usize }
                }
                _ => 0,
            };
            let end_idx = match end {
                Value::Null => len as usize,
                Value::Number(n) => {
                    let n = *n as i64;
                    if n < 0 { (len + n).max(0) as usize } else { n.min(len) as usize }
                }
                _ => len as usize,
            };
            let sliced = if start_idx < end_idx {
                &s[start_idx..end_idx]
            } else {
                ""
            };
            Value::String(sliced.to_string().into())
        } else {
            let chars: Vec<char> = s.chars().collect();
            let len = chars.len() as i64;
            let start_idx = match start {
                Value::Number(n) => {
                    let n = *n as i64;
                    if n < 0 { (len + n).max(0) as usize } else { n.min(len) as usize }
                }
                _ => 0,
            };
            let end_idx = match end {
                Value::Null => len as usize,
                Value::Number(n) => {
                    let n = *n as i64;
                    if n < 0 { (len + n).max(0) as usize } else { n.min(len) as usize }
                }
                _ => len as usize,
            };
            let sliced: String = if start_idx < end_idx {
                chars[start_idx..end_idx].iter().collect()
            } else {
                String::new()
            };
            Value::String(sliced.into())
        }
    } else {
        Value::Null
    }
}

pub fn substring(s: &Value, start: &Value, end: &Value) -> Value {
    if let Value::String(s) = s {
        if s.is_ascii() {
            let len = s.len();
            let start_idx = match start {
                Value::Number(n) => (*n as usize).min(len),
                _ => 0,
            };
            let end_idx = match end {
                Value::Null => len,
                Value::Number(n) => (*n as usize).min(len),
                _ => len,
            };
            let (ss, ee) = (start_idx.min(end_idx), start_idx.max(end_idx));
            Value::String(s[ss..ee].to_string().into())
        } else {
            let chars: Vec<char> = s.chars().collect();
            let len = chars.len();
            let start_idx = match start {
                Value::Number(n) => (*n as usize).min(len),
                _ => 0,
            };
            let end_idx = match end {
                Value::Null => len,
                Value::Number(n) => (*n as usize).min(len),
                _ => len,
            };
            let (ss, ee) = (start_idx.min(end_idx), start_idx.max(end_idx));
            Value::String(chars[ss..ee].iter().collect::<String>().into())
        }
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

pub fn replace(s: &Value, search: &Value, replacement: &Value) -> Value {
    if let Value::String(s) = s {
        let search_str = match search {
            Value::String(ss) => ss.to_string(),
            _ => return Value::String(Arc::clone(s)),
        };
        let replacement_str = match replacement {
            Value::String(ss) => ss.to_string(),
            _ => String::new(),
        };
        Value::String(s.replacen(&search_str, &replacement_str, 1).into())
    } else {
        Value::Null
    }
}

pub fn replace_all(s: &Value, search: &Value, replacement: &Value) -> Value {
    if let Value::String(s) = s {
        let search_str = match search {
            Value::String(ss) => ss.to_string(),
            _ => return Value::String(Arc::clone(s)),
        };
        let replacement_str = match replacement {
            Value::String(ss) => ss.to_string(),
            _ => String::new(),
        };
        Value::String(s.replace(&search_str, &replacement_str).into())
    } else {
        Value::Null
    }
}

pub fn char_at(s: &Value, idx: &Value) -> Value {
    if let Value::String(s) = s {
        let idx = match idx {
            Value::Number(n) => *n as usize,
            _ => 0,
        };
        if s.is_ascii() {
            s.as_bytes().get(idx)
                .map(|&b| Value::String((b as char).to_string().into()))
                .unwrap_or(Value::String("".into()))
        } else {
            s.chars().nth(idx)
                .map(|c| Value::String(c.to_string().into()))
                .unwrap_or(Value::String("".into()))
        }
    } else {
        Value::Null
    }
}

pub fn char_code_at(s: &Value, idx: &Value) -> Value {
    if let Value::String(s) = s {
        let idx = match idx {
            Value::Number(n) => *n as usize,
            _ => 0,
        };
        if s.is_ascii() {
            s.as_bytes().get(idx)
                .map(|&b| Value::Number(b as f64))
                .unwrap_or(Value::Number(f64::NAN))
        } else {
            s.chars().nth(idx)
                .map(|c| Value::Number(c as u32 as f64))
                .unwrap_or(Value::Number(f64::NAN))
        }
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

pub fn pad_start(s: &Value, target_len: &Value, pad: &Value) -> Value {
    if let Value::String(s) = s {
        let target_len = match target_len {
            Value::Number(n) => *n as usize,
            _ => return Value::String(Arc::clone(s)),
        };
        let pad_str = match pad {
            Value::String(p) => p.to_string(),
            Value::Null => " ".to_string(),
            _ => " ".to_string(),
        };
        let char_count = s.chars().count();
        if char_count >= target_len || pad_str.is_empty() {
            return Value::String(Arc::clone(s));
        }
        let needed = target_len - char_count;
        let padding: String = pad_str.chars().cycle().take(needed).collect();
        Value::String(format!("{}{}", padding, s).into())
    } else {
        Value::Null
    }
}

pub fn pad_end(s: &Value, target_len: &Value, pad: &Value) -> Value {
    if let Value::String(s) = s {
        let target_len = match target_len {
            Value::Number(n) => *n as usize,
            _ => return Value::String(Arc::clone(s)),
        };
        let pad_str = match pad {
            Value::String(p) => p.to_string(),
            Value::Null => " ".to_string(),
            _ => " ".to_string(),
        };
        let char_count = s.chars().count();
        if char_count >= target_len || pad_str.is_empty() {
            return Value::String(Arc::clone(s));
        }
        let needed = target_len - char_count;
        let padding: String = pad_str.chars().cycle().take(needed).collect();
        Value::String(format!("{}{}", s, padding).into())
    } else {
        Value::Null
    }
}
