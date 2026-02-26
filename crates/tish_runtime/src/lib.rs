//! Minimal runtime for Tish compiled output.
//!
//! Re-exports core types from tish_core and provides console, Math,
//! and other builtin functions for compiled Tish programs.

use std::fmt;

pub use tish_core::{
    Value, NativeFn,
    json_parse as core_json_parse, json_stringify as core_json_stringify,
    percent_decode, percent_encode,
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
    Value::Bool(args.first().map_or(false, |v| matches!(v, Value::Number(n) if n.is_finite())))
}

pub fn is_nan(args: &[Value]) -> Value {
    Value::Bool(args.first().map_or(true, |v| matches!(v, Value::Number(n) if n.is_nan()) || !matches!(v, Value::Number(_))))
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

use std::sync::Arc;

/// Get property from object/array by string key.
pub fn get_prop(obj: &Value, key: impl AsRef<str>) -> Value {
    let key = key.as_ref();
    match obj {
        Value::Object(map) => {
            let k: Arc<str> = key.into();
            map.get(&k).cloned().unwrap_or(Value::Null)
        }
        Value::Array(arr) => {
            if key == "length" {
                Value::Number(arr.len() as f64)
            } else if let Ok(idx) = key.parse::<usize>() {
                arr.get(idx).cloned().unwrap_or(Value::Null)
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
            arr.get(idx).cloned().unwrap_or(Value::Null)
        }
        Value::Object(map) => {
            let key: Arc<str> = match index {
                Value::Number(n) => n.to_string().into(),
                Value::String(s) => Arc::clone(s),
                _ => return Value::Null,
            };
            map.get(&key).cloned().unwrap_or(Value::Null)
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
        Value::Object(map) => map.contains_key(&key_str),
        Value::Array(arr) => {
            key_str.as_ref() == "length"
                || key_str
                    .parse::<usize>()
                    .ok()
                    .map(|i| i < arr.len())
                    .unwrap_or(false)
        }
        _ => false,
    };
    
    Value::Bool(result)
}
