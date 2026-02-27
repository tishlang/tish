//! Minimal runtime for Tish compiled output.
//!
//! Re-exports core types from tish_core and provides console, Math,
//! and other builtin functions for compiled Tish programs.

use std::fmt;

pub use tish_core::Value;

use tish_core::{
    json_parse as core_json_parse,
    json_stringify as core_json_stringify,
    percent_decode,
    percent_encode,
};

/// Error type for Tish throw/catch.
#[derive(Debug, Clone)]
pub enum TishError {
    Throw(Value),
}

impl fmt::Display for TishError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TishError::Throw(v) => write!(f, "{}", v.to_display_string()),
        }
    }
}

impl std::error::Error for TishError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum LogLevel {
    Debug = 0,
    Info = 1,
    Log = 2,
    Warn = 3,
    Error = 4,
}

fn get_log_level() -> LogLevel {
    match std::env::var("TISH_LOG_LEVEL").as_deref() {
        Ok("debug") => LogLevel::Debug,
        Ok("info") => LogLevel::Info,
        Ok("warn") => LogLevel::Warn,
        Ok("error") => LogLevel::Error,
        _ => LogLevel::Log,
    }
}

fn format_args(args: &[Value]) -> String {
    args.iter().map(Value::to_display_string).collect::<Vec<_>>().join(" ")
}

pub fn console_debug(args: &[Value]) {
    if get_log_level() <= LogLevel::Debug {
        println!("{}", format_args(args));
    }
}

pub fn console_info(args: &[Value]) {
    if get_log_level() <= LogLevel::Info {
        println!("{}", format_args(args));
    }
}

pub fn console_log(args: &[Value]) {
    if get_log_level() <= LogLevel::Log {
        println!("{}", format_args(args));
    }
}

pub fn console_warn(args: &[Value]) {
    if get_log_level() <= LogLevel::Warn {
        eprintln!("{}", format_args(args));
    }
}

pub fn console_error(args: &[Value]) {
    eprintln!("{}", format_args(args));
}

pub fn parse_int(args: &[Value]) -> Value {
    let s = args.first().map(Value::to_display_string).unwrap_or_default();
    let s = s.trim();
    let radix = args.get(1).and_then(|v| match v {
        Value::Number(n) => Some(*n as i32),
        _ => None,
    }).unwrap_or(10);
    
    if (2..=36).contains(&radix) {
        let prefix: String = s
            .chars()
            .take_while(|c| *c == '-' || *c == '+' || c.is_digit(radix as u32))
            .collect();
        if let Ok(n) = i64::from_str_radix(&prefix, radix as u32) {
            return Value::Number(n as f64);
        }
    }
    Value::Number(f64::NAN)
}

pub fn parse_float(args: &[Value]) -> Value {
    let s = args.first().map(Value::to_display_string).unwrap_or_default();
    Value::Number(s.trim().parse().unwrap_or(f64::NAN))
}

pub fn is_finite(args: &[Value]) -> Value {
    Value::Bool(args.first().is_some_and(|v| matches!(v, Value::Number(n) if n.is_finite())))
}

pub fn is_nan(args: &[Value]) -> Value {
    Value::Bool(args.first().is_none_or(|v| matches!(v, Value::Number(n) if n.is_nan()) || !matches!(v, Value::Number(_))))
}

pub fn decode_uri(args: &[Value]) -> Value {
    let s = args.first().map(Value::to_display_string).unwrap_or_default();
    Value::String(percent_decode(&s).unwrap_or(s).into())
}

pub fn encode_uri(args: &[Value]) -> Value {
    let s = args.first().map(Value::to_display_string).unwrap_or_default();
    Value::String(percent_encode(&s).into())
}

pub fn math_abs(args: &[Value]) -> Value {
    let n = extract_num(args.first()).unwrap_or(f64::NAN);
    Value::Number(n.abs())
}

pub fn math_sqrt(args: &[Value]) -> Value {
    let n = extract_num(args.first()).unwrap_or(f64::NAN);
    Value::Number(n.sqrt())
}

pub fn math_min(args: &[Value]) -> Value {
    let nums: Vec<f64> = args.iter().filter_map(|v| extract_num(Some(v))).collect();
    let n = nums.into_iter().fold(f64::INFINITY, f64::min);
    Value::Number(if n == f64::INFINITY { f64::NAN } else { n })
}

pub fn math_max(args: &[Value]) -> Value {
    let nums: Vec<f64> = args.iter().filter_map(|v| extract_num(Some(v))).collect();
    let n = nums.into_iter().fold(f64::NEG_INFINITY, f64::max);
    Value::Number(if n == f64::NEG_INFINITY { f64::NAN } else { n })
}

pub fn math_floor(args: &[Value]) -> Value {
    let n = extract_num(args.first()).unwrap_or(f64::NAN);
    Value::Number(n.floor())
}

pub fn math_ceil(args: &[Value]) -> Value {
    let n = extract_num(args.first()).unwrap_or(f64::NAN);
    Value::Number(n.ceil())
}

pub fn math_round(args: &[Value]) -> Value {
    let n = extract_num(args.first()).unwrap_or(f64::NAN);
    Value::Number(n.round())
}

pub fn json_stringify(args: &[Value]) -> Value {
    let v = args.first().cloned().unwrap_or(Value::Null);
    Value::String(core_json_stringify(&v).into())
}

pub fn json_parse(args: &[Value]) -> Value {
    let s = args.first().map(|v| v.to_display_string()).unwrap_or_default();
    core_json_parse(&s).unwrap_or(Value::Null)
}

fn extract_num(v: Option<&Value>) -> Option<f64> {
    v.and_then(|v| match v { Value::Number(n) => Some(*n), _ => None })
}

// ============== New Math Functions ==============

pub fn math_random(_args: &[Value]) -> Value {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    let random = RandomState::new().build_hasher().finish() as f64 / u64::MAX as f64;
    Value::Number(random)
}

pub fn math_pow(args: &[Value]) -> Value {
    let base = extract_num(args.first()).unwrap_or(f64::NAN);
    let exp = extract_num(args.get(1)).unwrap_or(f64::NAN);
    Value::Number(base.powf(exp))
}

pub fn math_sin(args: &[Value]) -> Value {
    let n = extract_num(args.first()).unwrap_or(f64::NAN);
    Value::Number(n.sin())
}

pub fn math_cos(args: &[Value]) -> Value {
    let n = extract_num(args.first()).unwrap_or(f64::NAN);
    Value::Number(n.cos())
}

pub fn math_tan(args: &[Value]) -> Value {
    let n = extract_num(args.first()).unwrap_or(f64::NAN);
    Value::Number(n.tan())
}

pub fn math_log(args: &[Value]) -> Value {
    let n = extract_num(args.first()).unwrap_or(f64::NAN);
    Value::Number(n.ln())
}

pub fn math_exp(args: &[Value]) -> Value {
    let n = extract_num(args.first()).unwrap_or(f64::NAN);
    Value::Number(n.exp())
}

pub fn math_sign(args: &[Value]) -> Value {
    let n = extract_num(args.first()).unwrap_or(f64::NAN);
    let sign = if n.is_nan() { f64::NAN } else if n > 0.0 { 1.0 } else if n < 0.0 { -1.0 } else { 0.0 };
    Value::Number(sign)
}

pub fn math_trunc(args: &[Value]) -> Value {
    let n = extract_num(args.first()).unwrap_or(f64::NAN);
    Value::Number(n.trunc())
}

// ============== Date Functions ==============

pub fn date_now(_args: &[Value]) -> Value {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as f64)
        .unwrap_or(0.0);
    Value::Number(now)
}

// ============== Array/String Static Functions ==============

pub fn array_is_array(args: &[Value]) -> Value {
    Value::Bool(matches!(args.first(), Some(Value::Array(_))))
}

pub fn string_from_char_code(args: &[Value]) -> Value {
    let s: String = args.iter().filter_map(|v| match v {
        Value::Number(n) => char::from_u32(*n as u32),
        _ => None,
    }).collect();
    Value::String(s.into())
}

// ============== Process Functions ==============

#[cfg(feature = "process")]
pub fn process_exit(args: &[Value]) -> Value {
    let code = args.first().and_then(|v| match v {
        Value::Number(n) => Some(*n as i32),
        _ => None,
    }).unwrap_or(0);
    std::process::exit(code);
}

#[cfg(feature = "process")]
pub fn process_cwd(_args: &[Value]) -> Value {
    let cwd = std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    Value::String(cwd.into())
}

// ============== File I/O Functions ==============

#[cfg(feature = "fs")]
pub fn read_file(args: &[Value]) -> Value {
    let path = args.first().map(|v| v.to_display_string()).unwrap_or_default();
    match std::fs::read_to_string(&path) {
        Ok(content) => Value::String(content.into()),
        Err(e) => {
            let mut obj = std::collections::HashMap::new();
            obj.insert(std::sync::Arc::from("error"), Value::String(e.to_string().into()));
            Value::Object(std::rc::Rc::new(std::cell::RefCell::new(obj)))
        }
    }
}

#[cfg(feature = "fs")]
pub fn write_file(args: &[Value]) -> Value {
    let path = args.first().map(|v| v.to_display_string()).unwrap_or_default();
    let content = args.get(1).map(|v| v.to_display_string()).unwrap_or_default();
    match std::fs::write(&path, &content) {
        Ok(()) => Value::Bool(true),
        Err(e) => {
            let mut obj = std::collections::HashMap::new();
            obj.insert(std::sync::Arc::from("error"), Value::String(e.to_string().into()));
            Value::Object(std::rc::Rc::new(std::cell::RefCell::new(obj)))
        }
    }
}

#[cfg(feature = "fs")]
pub fn file_exists(args: &[Value]) -> Value {
    let path = args.first().map(|v| v.to_display_string()).unwrap_or_default();
    Value::Bool(std::path::Path::new(&path).exists())
}

#[cfg(feature = "fs")]
pub fn read_dir(args: &[Value]) -> Value {
    use std::cell::RefCell;
    use std::rc::Rc;
    let path = args.first().map(|v| v.to_display_string()).unwrap_or_else(|| ".".to_string());
    match std::fs::read_dir(&path) {
        Ok(entries) => {
            let files: Vec<Value> = entries
                .filter_map(|e| e.ok())
                .map(|e| Value::String(e.file_name().to_string_lossy().into()))
                .collect();
            Value::Array(Rc::new(RefCell::new(files)))
        }
        Err(e) => {
            let mut obj = std::collections::HashMap::new();
            obj.insert(std::sync::Arc::from("error"), Value::String(e.to_string().into()));
            Value::Object(std::rc::Rc::new(std::cell::RefCell::new(obj)))
        }
    }
}

#[cfg(feature = "fs")]
pub fn mkdir(args: &[Value]) -> Value {
    let path = args.first().map(|v| v.to_display_string()).unwrap_or_default();
    match std::fs::create_dir_all(&path) {
        Ok(()) => Value::Bool(true),
        Err(e) => {
            let mut obj = std::collections::HashMap::new();
            obj.insert(std::sync::Arc::from("error"), Value::String(e.to_string().into()));
            Value::Object(std::rc::Rc::new(std::cell::RefCell::new(obj)))
        }
    }
}

use std::sync::Arc;

/// Get property from object/array by string key.
pub fn get_prop(obj: &Value, key: impl AsRef<str>) -> Value {
    let key = key.as_ref();
    match obj {
        Value::Object(map) => {
            let k: Arc<str> = key.into();
            map.borrow().get(&k).cloned().unwrap_or(Value::Null)
        }
        Value::Array(arr) => {
            if key == "length" {
                Value::Number(arr.borrow().len() as f64)
            } else if let Ok(idx) = key.parse::<usize>() {
                arr.borrow().get(idx).cloned().unwrap_or(Value::Null)
            } else {
                Value::Null
            }
        }
        Value::String(s) => {
            if key == "length" {
                Value::Number(s.chars().count() as f64)
            } else {
                Value::Null
            }
        }
        _ => Value::Null,
    }
}

/// Get index from array or object.
pub fn get_index(obj: &Value, index: &Value) -> Value {
    match obj {
        Value::Array(arr) => {
            let idx = match index {
                Value::Number(n) => *n as usize,
                _ => return Value::Null,
            };
            arr.borrow().get(idx).cloned().unwrap_or(Value::Null)
        }
        Value::Object(map) => {
            let key: Arc<str> = match index {
                Value::Number(n) => n.to_string().into(),
                Value::String(s) => Arc::clone(s),
                _ => return Value::Null,
            };
            map.borrow().get(&key).cloned().unwrap_or(Value::Null)
        }
        _ => Value::Null,
    }
}

/// 'in' operator: check if key exists in object/array.
pub fn in_operator(key: &Value, obj: &Value) -> Value {
    let key_str: Arc<str> = match key {
        Value::String(s) => Arc::clone(s),
        Value::Number(n) => n.to_string().into(),
        _ => return Value::Bool(false),
    };
    
    let result = match obj {
        Value::Object(map) => map.borrow().contains_key(&key_str),
        Value::Array(arr) => {
            key_str.as_ref() == "length"
                || key_str
                    .parse::<usize>()
                    .ok()
                    .map(|i| i < arr.borrow().len())
                    .unwrap_or(false)
        }
        _ => false,
    };
    
    Value::Bool(result)
}

use std::cell::RefCell;
use std::rc::Rc;

// ============== Array Methods ==============

pub fn array_push(arr: &Value, args: &[Value]) -> Value {
    if let Value::Array(arr) = arr {
        let mut arr_mut = arr.borrow_mut();
        for v in args {
            arr_mut.push(v.clone());
        }
        Value::Number(arr_mut.len() as f64)
    } else {
        Value::Null
    }
}

pub fn array_pop(arr: &Value) -> Value {
    if let Value::Array(arr) = arr {
        arr.borrow_mut().pop().unwrap_or(Value::Null)
    } else {
        Value::Null
    }
}

pub fn array_shift(arr: &Value) -> Value {
    if let Value::Array(arr) = arr {
        let mut arr_mut = arr.borrow_mut();
        if arr_mut.is_empty() {
            Value::Null
        } else {
            arr_mut.remove(0)
        }
    } else {
        Value::Null
    }
}

pub fn array_unshift(arr: &Value, args: &[Value]) -> Value {
    if let Value::Array(arr) = arr {
        let mut arr_mut = arr.borrow_mut();
        for (i, v) in args.iter().enumerate() {
            arr_mut.insert(i, v.clone());
        }
        Value::Number(arr_mut.len() as f64)
    } else {
        Value::Null
    }
}

pub fn array_index_of(arr: &Value, search: &Value) -> Value {
    if let Value::Array(arr) = arr {
        let arr_borrow = arr.borrow();
        for (i, v) in arr_borrow.iter().enumerate() {
            if v.strict_eq(search) {
                return Value::Number(i as f64);
            }
        }
    }
    Value::Number(-1.0)
}

pub fn array_includes(arr: &Value, search: &Value) -> Value {
    if let Value::Array(arr) = arr {
        let arr_borrow = arr.borrow();
        for v in arr_borrow.iter() {
            if v.strict_eq(search) {
                return Value::Bool(true);
            }
        }
    }
    Value::Bool(false)
}

pub fn array_join(arr: &Value, sep: &Value) -> Value {
    if let Value::Array(arr) = arr {
        let separator = match sep {
            Value::String(s) => s.to_string(),
            _ => ",".to_string(),
        };
        let arr_borrow = arr.borrow();
        let parts: Vec<String> = arr_borrow.iter().map(|v| v.to_display_string()).collect();
        Value::String(parts.join(&separator).into())
    } else {
        Value::Null
    }
}

pub fn array_reverse(arr: &Value) -> Value {
    if let Value::Array(arr) = arr {
        arr.borrow_mut().reverse();
        Value::Array(Rc::clone(arr))
    } else {
        Value::Null
    }
}

pub fn array_sort(arr: &Value, comparator: Option<&Value>) -> Value {
    if let Value::Array(arr) = arr {
        let mut arr_mut = arr.borrow_mut();
        
        if let Some(Value::Function(cmp_fn)) = comparator {
            let mut indices: Vec<usize> = (0..arr_mut.len()).collect();
            let arr_clone: Vec<Value> = arr_mut.clone();
            
            indices.sort_by(|&a, &b| {
                let va = &arr_clone[a];
                let vb = &arr_clone[b];
                let result = cmp_fn(&[va.clone(), vb.clone()]);
                match result {
                    Value::Number(n) => {
                        if n < 0.0 {
                            std::cmp::Ordering::Less
                        } else if n > 0.0 {
                            std::cmp::Ordering::Greater
                        } else {
                            std::cmp::Ordering::Equal
                        }
                    }
                    _ => std::cmp::Ordering::Equal,
                }
            });
            
            let sorted: Vec<Value> = indices.iter().map(|&i| arr_clone[i].clone()).collect();
            *arr_mut = sorted;
        } else {
            arr_mut.sort_by(|a, b| {
                let sa = a.to_display_string();
                let sb = b.to_display_string();
                sa.cmp(&sb)
            });
        }
        drop(arr_mut);
        Value::Array(Rc::clone(arr))
    } else {
        Value::Null
    }
}

pub fn array_splice(arr: &Value, start: &Value, delete_count: Option<&Value>, items: &[Value]) -> Value {
    if let Value::Array(arr) = arr {
        let mut arr_mut = arr.borrow_mut();
        let len = arr_mut.len() as i64;
        
        let start_idx = match start {
            Value::Number(n) => {
                let n = *n as i64;
                if n < 0 { (len + n).max(0) as usize } else { n.min(len) as usize }
            }
            _ => 0,
        };
        
        let del_count = match delete_count {
            Some(Value::Number(n)) => (*n as i64).max(0) as usize,
            _ => (len as usize).saturating_sub(start_idx),
        };
        
        let actual_delete = del_count.min(arr_mut.len().saturating_sub(start_idx));
        let removed: Vec<Value> = arr_mut.drain(start_idx..start_idx + actual_delete).collect();
        
        for (i, item) in items.iter().enumerate() {
            arr_mut.insert(start_idx + i, item.clone());
        }
        
        Value::Array(Rc::new(RefCell::new(removed)))
    } else {
        Value::Null
    }
}

pub fn array_slice(arr: &Value, start: &Value, end: &Value) -> Value {
    if let Value::Array(arr) = arr {
        let arr_borrow = arr.borrow();
        let len = arr_borrow.len() as i64;
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
        let sliced: Vec<Value> = if start_idx < end_idx {
            arr_borrow[start_idx..end_idx].to_vec()
        } else {
            vec![]
        };
        Value::Array(Rc::new(RefCell::new(sliced)))
    } else {
        Value::Null
    }
}

pub fn array_concat(arr: &Value, args: &[Value]) -> Value {
    if let Value::Array(arr) = arr {
        let mut result = arr.borrow().clone();
        for v in args {
            if let Value::Array(other) = v {
                result.extend(other.borrow().iter().cloned());
            } else {
                result.push(v.clone());
            }
        }
        Value::Array(Rc::new(RefCell::new(result)))
    } else {
        Value::Null
    }
}

// ============== String Methods ==============

pub fn string_index_of(s: &Value, search: &Value) -> Value {
    if let (Value::String(s), Value::String(search)) = (s, search) {
        Value::Number(s.find(search.as_ref()).map(|i| i as f64).unwrap_or(-1.0))
    } else {
        Value::Number(-1.0)
    }
}

pub fn string_includes(s: &Value, search: &Value) -> Value {
    if let (Value::String(s), Value::String(search)) = (s, search) {
        Value::Bool(s.contains(search.as_ref()))
    } else {
        Value::Bool(false)
    }
}

pub fn string_slice(s: &Value, start: &Value, end: &Value) -> Value {
    if let Value::String(s) = s {
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
    } else {
        Value::Null
    }
}

pub fn string_substring(s: &Value, start: &Value, end: &Value) -> Value {
    if let Value::String(s) = s {
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
        let (s, e) = (start_idx.min(end_idx), start_idx.max(end_idx));
        Value::String(chars[s..e].iter().collect::<String>().into())
    } else {
        Value::Null
    }
}

pub fn string_split(s: &Value, sep: &Value) -> Value {
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

pub fn string_trim(s: &Value) -> Value {
    if let Value::String(s) = s {
        Value::String(s.trim().into())
    } else {
        Value::Null
    }
}

pub fn string_to_upper_case(s: &Value) -> Value {
    if let Value::String(s) = s {
        Value::String(s.to_uppercase().into())
    } else {
        Value::Null
    }
}

pub fn string_to_lower_case(s: &Value) -> Value {
    if let Value::String(s) = s {
        Value::String(s.to_lowercase().into())
    } else {
        Value::Null
    }
}

pub fn string_starts_with(s: &Value, search: &Value) -> Value {
    if let (Value::String(s), Value::String(search)) = (s, search) {
        Value::Bool(s.starts_with(search.as_ref()))
    } else {
        Value::Bool(false)
    }
}

pub fn string_ends_with(s: &Value, search: &Value) -> Value {
    if let (Value::String(s), Value::String(search)) = (s, search) {
        Value::Bool(s.ends_with(search.as_ref()))
    } else {
        Value::Bool(false)
    }
}

pub fn string_replace(s: &Value, search: &Value, replacement: &Value) -> Value {
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

pub fn string_replace_all(s: &Value, search: &Value, replacement: &Value) -> Value {
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

pub fn string_char_at(s: &Value, idx: &Value) -> Value {
    if let Value::String(s) = s {
        let idx = match idx {
            Value::Number(n) => *n as usize,
            _ => 0,
        };
        let chars: Vec<char> = s.chars().collect();
        chars.get(idx)
            .map(|c| Value::String(c.to_string().into()))
            .unwrap_or(Value::String("".into()))
    } else {
        Value::Null
    }
}

pub fn string_char_code_at(s: &Value, idx: &Value) -> Value {
    if let Value::String(s) = s {
        let idx = match idx {
            Value::Number(n) => *n as usize,
            _ => 0,
        };
        let chars: Vec<char> = s.chars().collect();
        chars.get(idx)
            .map(|c| Value::Number(*c as u32 as f64))
            .unwrap_or(Value::Number(f64::NAN))
    } else {
        Value::Null
    }
}

pub fn string_repeat(s: &Value, count: &Value) -> Value {
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

pub fn string_pad_start(s: &Value, target_len: &Value, pad: &Value) -> Value {
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
        let chars: Vec<char> = s.chars().collect();
        if chars.len() >= target_len || pad_str.is_empty() {
            return Value::String(Arc::clone(s));
        }
        let needed = target_len - chars.len();
        let padding: String = pad_str.chars().cycle().take(needed).collect();
        Value::String(format!("{}{}", padding, s).into())
    } else {
        Value::Null
    }
}

pub fn string_pad_end(s: &Value, target_len: &Value, pad: &Value) -> Value {
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
        let chars: Vec<char> = s.chars().collect();
        if chars.len() >= target_len || pad_str.is_empty() {
            return Value::String(Arc::clone(s));
        }
        let needed = target_len - chars.len();
        let padding: String = pad_str.chars().cycle().take(needed).collect();
        Value::String(format!("{}{}", s, padding).into())
    } else {
        Value::Null
    }
}

// ============== HTTP Support ==============

#[cfg(feature = "http")]
mod http;

#[cfg(feature = "http")]
pub use http::{fetch as http_fetch, fetch_all as http_fetch_all, serve as http_serve};

// ============== RegExp Support ==============

#[cfg(feature = "regex")]
pub use tish_core::{TishRegExp, RegExpFlags};

#[cfg(feature = "regex")]
pub fn regexp_new(args: &[Value]) -> Value {
    let pattern = match args.first() {
        Some(Value::String(s)) => s.to_string(),
        Some(v) => v.to_display_string(),
        None => String::new(),
    };

    let flags = match args.get(1) {
        Some(Value::String(s)) => s.to_string(),
        Some(Value::Null) | None => String::new(),
        Some(v) => v.to_display_string(),
    };

    match TishRegExp::new(&pattern, &flags) {
        Ok(re) => Value::RegExp(Rc::new(RefCell::new(re))),
        Err(e) => {
            eprintln!("RegExp error: {}", e);
            Value::Null
        }
    }
}

#[cfg(feature = "regex")]
pub fn regexp_test(re: &Value, input: &Value) -> Value {
    if let Value::RegExp(re) = re {
        let input_str = input.to_display_string();
        Value::Bool(re.borrow_mut().test(&input_str))
    } else {
        Value::Bool(false)
    }
}

#[cfg(feature = "regex")]
pub fn regexp_exec(re: &Value, input: &Value) -> Value {
    if let Value::RegExp(re) = re {
        let input_str = input.to_display_string();
        regexp_exec_impl(&mut re.borrow_mut(), &input_str)
    } else {
        Value::Null
    }
}

#[cfg(feature = "regex")]
fn regexp_exec_impl(re: &mut tish_core::TishRegExp, input: &str) -> Value {
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

            let mut result = Vec::new();
            result.push(Value::String(full_match.as_str().into()));
            
            for i in 1..caps.len() {
                match caps.get(i) {
                    Some(m) => result.push(Value::String(m.as_str().into())),
                    None => result.push(Value::Null),
                }
            }

            if re.flags.global || re.flags.sticky {
                let match_end_chars = input[..byte_start + full_match.end()].chars().count();
                if full_match.start() == full_match.end() {
                    re.last_index = match_end_chars + 1;
                } else {
                    re.last_index = match_end_chars;
                }
            }

            Value::Array(Rc::new(RefCell::new(result)))
        }
        Ok(None) | Err(_) => {
            if re.flags.global || re.flags.sticky {
                re.last_index = 0;
            }
            Value::Null
        }
    }
}

#[cfg(feature = "regex")]
pub fn string_split_regex(s: &Value, separator: &Value, limit: Option<usize>) -> Value {
    let input = match s {
        Value::String(s) => s.as_ref(),
        _ => return Value::Array(Rc::new(RefCell::new(vec![s.clone()]))),
    };
    
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
                        if result.len() >= max - 1 { break; }
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
        _ => Value::Array(Rc::new(RefCell::new(vec![Value::String(input.into())]))),
    }
}

#[cfg(feature = "regex")]
pub fn string_match_regex(s: &Value, regexp: &Value) -> Value {
    let input = match s {
        Value::String(s) => s.as_ref(),
        _ => return Value::Null,
    };

    match regexp {
        Value::RegExp(re) => {
            let mut re = re.borrow_mut();
            
            if re.flags.global {
                let mut matches = Vec::new();
                re.last_index = 0;
                
                loop {
                    match re.regex.find_from_pos(input, re.last_index) {
                        Ok(Some(m)) => {
                            matches.push(Value::String(m.as_str().into()));
                            if m.start() == m.end() {
                                re.last_index = m.end() + 1;
                            } else {
                                re.last_index = m.end();
                            }
                            if re.last_index > input.len() { break; }
                        }
                        _ => break,
                    }
                }
                
                re.last_index = 0;
                
                if matches.is_empty() {
                    Value::Null
                } else {
                    Value::Array(Rc::new(RefCell::new(matches)))
                }
            } else {
                regexp_exec_impl(&mut re, input)
            }
        }
        Value::String(pattern) => {
            match tish_core::TishRegExp::new(pattern, "") {
                Ok(mut re) => regexp_exec_impl(&mut re, input),
                Err(_) => Value::Null,
            }
        }
        _ => Value::Null,
    }
}

#[cfg(feature = "regex")]
pub fn string_replace_regex(s: &Value, search: &Value, replacement: &Value) -> Value {
    let input = match s {
        Value::String(s) => s.as_ref(),
        _ => return s.clone(),
    };
    
    let replacement_str = replacement.to_display_string();

    match search {
        Value::RegExp(re) => {
            let re = re.borrow();
            
            if re.flags.global {
                match re.regex.replace_all(input, replacement_str.as_str()) {
                    std::borrow::Cow::Borrowed(s) => Value::String(s.into()),
                    std::borrow::Cow::Owned(s) => Value::String(s.into()),
                }
            } else {
                match re.regex.replace(input, replacement_str.as_str()) {
                    std::borrow::Cow::Borrowed(s) => Value::String(s.into()),
                    std::borrow::Cow::Owned(s) => Value::String(s.into()),
                }
            }
        }
        Value::String(pattern) => {
            Value::String(input.replacen(pattern.as_ref(), &replacement_str, 1).into())
        }
        _ => Value::String(input.into()),
    }
}

#[cfg(feature = "regex")]
pub fn string_search_regex(s: &Value, regexp: &Value) -> Value {
    let input = match s {
        Value::String(s) => s.as_ref(),
        _ => return Value::Number(-1.0),
    };

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
        Value::String(pattern) => {
            match tish_core::TishRegExp::new(pattern, "") {
                Ok(re) => match re.regex.find(input) {
                    Ok(Some(m)) => {
                        let char_index = input[..m.start()].chars().count();
                        Value::Number(char_index as f64)
                    }
                    _ => Value::Number(-1.0),
                },
                Err(_) => Value::Number(-1.0),
            }
        }
        _ => Value::Number(-1.0),
    }
}
