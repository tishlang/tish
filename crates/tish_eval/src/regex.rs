//! JavaScript-compatible regular expression support for Tish.
//!
//! Re-exports core types from tish_core and provides interpreter-specific functionality.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

pub use tish_core::{RegExpFlags, TishRegExp};

use crate::value::Value;

/// RegExp.prototype.exec(string) - returns match object (array-like with index) or null
pub fn regexp_exec(re: &mut TishRegExp, input: &str) -> Value {
    let start = if re.flags.global || re.flags.sticky {
        re.last_index
    } else {
        0
    };

    let char_count = input.chars().count();
    if start > char_count {
        if re.flags.global || re.flags.sticky {
            re.last_index = 0;
        }
        return Value::Null;
    }

    let byte_start: usize = input.chars().take(start).map(|c| c.len_utf8()).sum();
    let search_str = &input[byte_start..];

    match re.regex.captures(search_str) {
        Ok(Some(caps)) => {
            let full_match = caps.get(0).unwrap();

            if re.flags.sticky && full_match.start() != 0 {
                re.last_index = 0;
                return Value::Null;
            }

            let match_byte_start = byte_start + full_match.start();
            let match_char_index = input[..match_byte_start].chars().count();

            let mut obj: HashMap<Arc<str>, Value> = HashMap::new();
            obj.insert(Arc::from("0"), Value::String(full_match.as_str().into()));
            for i in 1..caps.len() {
                let val = match caps.get(i) {
                    Some(m) => Value::String(m.as_str().into()),
                    None => Value::Null,
                };
                obj.insert(Arc::from(i.to_string().as_str()), val);
            }
            obj.insert(Arc::from("index"), Value::Number(match_char_index as f64));

            if re.flags.global || re.flags.sticky {
                let match_end_chars = input[..byte_start + full_match.end()].chars().count();
                re.last_index = if full_match.start() == full_match.end() {
                    match_end_chars + 1
                } else {
                    match_end_chars
                };
            }

            Value::Object(Rc::new(RefCell::new(obj)))
        }
        Ok(None) | Err(_) => {
            if re.flags.global || re.flags.sticky {
                re.last_index = 0;
            }
            Value::Null
        }
    }
}

/// Create a RegExp Value from pattern and flags
pub fn create_regexp(pattern: &str, flags: &str) -> Result<Value, String> {
    let re = TishRegExp::new(pattern, flags)?;
    Ok(Value::RegExp(Rc::new(RefCell::new(re))))
}

/// RegExp constructor function - handles `new RegExp(pattern, flags)` or `RegExp(pattern, flags)`
pub fn regexp_constructor(args: &[Value]) -> Result<Value, String> {
    let pattern = match args.first() {
        Some(Value::String(s)) => s.to_string(),
        Some(Value::RegExp(re)) => {
            if args.get(1).is_none() {
                let re = re.borrow();
                return create_regexp(&re.source, &re.flags_string());
            }
            re.borrow().source.clone()
        }
        Some(v) => v.to_string(),
        None => String::new(),
    };

    let flags = match args.get(1) {
        Some(Value::String(s)) => s.to_string(),
        Some(Value::Null) | None => String::new(),
        Some(v) => v.to_string(),
    };

    create_regexp(&pattern, &flags)
}

// ============== String methods with regex support ==============

/// String.prototype.match(regexp) - returns array of matches or null
pub fn string_match(input: &str, regexp: &Value) -> Value {
    match regexp {
        Value::RegExp(re) => {
            let mut re = re.borrow_mut();

            if re.flags.global {
                let mut matches = Vec::new();
                re.last_index = 0;

                while let Ok(Some(m)) = re.regex.find_from_pos(input, re.last_index) {
                    matches.push(Value::String(m.as_str().into()));
                    if m.start() == m.end() {
                        re.last_index = m.end() + 1;
                    } else {
                        re.last_index = m.end();
                    }
                    if re.last_index > input.len() {
                        break;
                    }
                }

                re.last_index = 0;

                if matches.is_empty() {
                    Value::Null
                } else {
                    Value::Array(Rc::new(RefCell::new(matches)))
                }
            } else {
                regexp_exec(&mut re, input)
            }
        }
        Value::String(pattern) => match TishRegExp::new(pattern, "") {
            Ok(mut re) => regexp_exec(&mut re, input),
            Err(_) => Value::Null,
        },
        _ => Value::Null,
    }
}

/// String.prototype.replace(searchValue, replaceValue)
pub fn string_replace(input: &str, search: &Value, replace: &Value) -> Value {
    let replacement = match replace {
        Value::String(s) => s.to_string(),
        v => v.to_string(),
    };

    match search {
        Value::RegExp(re) => {
            let re = re.borrow();
            if re.flags.global {
                match re.regex.replace_all(input, replacement.as_str()) {
                    std::borrow::Cow::Borrowed(s) => Value::String(s.into()),
                    std::borrow::Cow::Owned(s) => Value::String(s.into()),
                }
            } else {
                match re.regex.replace(input, replacement.as_str()) {
                    std::borrow::Cow::Borrowed(s) => Value::String(s.into()),
                    std::borrow::Cow::Owned(s) => Value::String(s.into()),
                }
            }
        }
        Value::String(pattern) => {
            Value::String(input.replacen(pattern.as_ref(), &replacement, 1).into())
        }
        _ => Value::String(input.into()),
    }
}

/// Replace regex matches using a callback. Callback receives (match, g1, g2, ..., index, fullString).
pub fn string_replace_regex_with_fn<F>(
    input: &str,
    re: &TishRegExp,
    invoke: &mut F,
) -> Result<Value, String>
where
    F: FnMut(&[Value]) -> Result<String, String>,
{
    let limit = if re.flags.global { usize::MAX } else { 1 };
    let mut result = String::new();
    let mut last_end: usize = 0;
    let mut count = 0usize;

    for cap_result in re.regex.captures_iter(input) {
        if count >= limit {
            break;
        }
        let caps = cap_result.map_err(|e| format!("Regex error: {}", e))?;
        let full = caps.get(0).unwrap();
        let match_str = full.as_str();
        let byte_start = full.start();
        let char_index = input[..byte_start].chars().count();

        let mut args = vec![Value::String(match_str.into())];
        for i in 1..caps.len() {
            let val = match caps.get(i) {
                Some(m) => Value::String(m.as_str().into()),
                None => Value::Null,
            };
            args.push(val);
        }
        args.push(Value::Number(char_index as f64));
        args.push(Value::String(input.into()));

        let repl = invoke(&args)?;
        result.push_str(&input[last_end..byte_start]);
        result.push_str(&repl);
        last_end = full.end();
        count += 1;
    }

    result.push_str(&input[last_end..]);
    Ok(Value::String(result.into()))
}

/// String.prototype.search(regexp) - returns index of first match or -1
pub fn string_search(input: &str, regexp: &Value) -> Value {
    match regexp {
        Value::RegExp(re) => {
            let re = re.borrow();
            match re.regex.find(input) {
                Ok(Some(m)) => {
                    let char_index = input[..m.start()].chars().count();
                    Value::Number(char_index as f64)
                }
                _ => Value::Number(-1.0),
            }
        }
        Value::String(pattern) => match TishRegExp::new(pattern, "") {
            Ok(re) => match re.regex.find(input) {
                Ok(Some(m)) => {
                    let char_index = input[..m.start()].chars().count();
                    Value::Number(char_index as f64)
                }
                _ => Value::Number(-1.0),
            },
            Err(_) => Value::Number(-1.0),
        },
        _ => Value::Number(-1.0),
    }
}

/// String.prototype.split(separator, limit) - split string by regex or string
pub fn string_split(input: &str, separator: &Value, limit: Option<usize>) -> Value {
    let max = limit.unwrap_or(usize::MAX);

    if max == 0 {
        return Value::Array(Rc::new(RefCell::new(Vec::new())));
    }

    match separator {
        Value::RegExp(re) => {
            let re = re.borrow();
            let mut result = Vec::new();
            let mut last_end = 0;

            for mat in re.regex.find_iter(input) {
                match mat {
                    Ok(m) => {
                        if result.len() >= max - 1 {
                            break;
                        }
                        result.push(Value::String(input[last_end..m.start()].into()));
                        last_end = m.end();
                    }
                    Err(_) => break,
                }
            }

            if result.len() < max {
                result.push(Value::String(input[last_end..].into()));
            }

            Value::Array(Rc::new(RefCell::new(result)))
        }
        Value::String(sep) => {
            let parts: Vec<Value> = input
                .splitn(max, sep.as_ref())
                .map(|s| Value::String(s.into()))
                .collect();
            Value::Array(Rc::new(RefCell::new(parts)))
        }
        Value::Null => Value::Array(Rc::new(RefCell::new(vec![Value::String(input.into())]))),
        _ => {
            let sep_str = separator.to_string();
            let parts: Vec<Value> = input
                .splitn(max, &sep_str)
                .map(|s| Value::String(s.into()))
                .collect();
            Value::Array(Rc::new(RefCell::new(parts)))
        }
    }
}
