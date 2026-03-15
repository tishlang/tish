//! Minimal runtime for Tish compiled output.
//!
//! Re-exports core types from tish_core and provides console, Math,
//! and other builtin functions for compiled Tish programs.

use std::fmt;
use std::sync::OnceLock;
use tish_builtins::helpers::extract_num;
#[cfg(feature = "fs")]
use tish_builtins::helpers::make_error_value;

pub use tish_core::Value;

// Re-export array methods from tish_builtins
pub use tish_builtins::array::{
    push as array_push_impl,
    pop as array_pop,
    shift as array_shift,
    unshift as array_unshift_impl,
    index_of as array_index_of_impl,
    includes as array_includes_impl,
    join as array_join_impl,
    reverse as array_reverse,
    shuffle as array_shuffle,
    splice as array_splice_impl,
    slice as array_slice_impl,
    concat as array_concat_impl,
    flat as array_flat_impl,
    map as array_map,
    filter as array_filter,
    reduce as array_reduce,
    for_each as array_for_each,
    find as array_find,
    find_index as array_find_index,
    some as array_some,
    every as array_every,
    flat_map as array_flat_map,
    sort_default as array_sort_default,
    sort_with_comparator as array_sort_with_comparator,
    sort_numeric_asc as array_sort_numeric_asc,
    sort_numeric_desc as array_sort_numeric_desc,
};

// Re-export string methods from tish_builtins
pub use tish_builtins::string::{
    index_of as string_index_of_impl,
    includes as string_includes_impl,
    slice as string_slice_impl,
    substring as string_substring_impl,
    split as string_split_impl,
    trim as string_trim,
    to_upper_case as string_to_upper_case,
    to_lower_case as string_to_lower_case,
    starts_with as string_starts_with_impl,
    ends_with as string_ends_with_impl,
    replace as string_replace_impl,
    replace_all as string_replace_all_impl,
    char_at as string_char_at_impl,
    char_code_at as string_char_code_at_impl,
    repeat as string_repeat_impl,
    pad_start as string_pad_start_impl,
    pad_end as string_pad_end_impl,
};

// Wrapper functions to maintain API compatibility
pub fn array_push(arr: &Value, args: &[Value]) -> Value { array_push_impl(arr, args) }
pub fn array_unshift(arr: &Value, args: &[Value]) -> Value { array_unshift_impl(arr, args) }
pub fn array_index_of(arr: &Value, search: &Value) -> Value { array_index_of_impl(arr, search) }
pub fn array_includes(arr: &Value, search: &Value, from: &Value) -> Value {
    array_includes_impl(arr, search, Some(from))
}
pub fn array_join(arr: &Value, sep: &Value) -> Value { array_join_impl(arr, sep) }
pub fn array_splice(arr: &Value, start: &Value, delete_count: Option<&Value>, items: &[Value]) -> Value {
    array_splice_impl(arr, start, delete_count, items)
}
pub fn array_slice(arr: &Value, start: &Value, end: &Value) -> Value { array_slice_impl(arr, start, end) }
pub fn array_concat(arr: &Value, args: &[Value]) -> Value { array_concat_impl(arr, args) }
pub fn array_flat(arr: &Value, depth: &Value) -> Value { array_flat_impl(arr, depth) }

pub fn array_sort(arr: &Value, comparator: Option<&Value>) -> Value {
    match comparator {
        Some(cmp) => array_sort_with_comparator(arr, cmp),
        None => array_sort_default(arr),
    }
}

pub fn string_index_of(s: &Value, search: &Value, from: &Value) -> Value {
    string_index_of_impl(s, search, Some(from))
}
pub fn string_includes(s: &Value, search: &Value, from: &Value) -> Value {
    string_includes_impl(s, search, Some(from))
}
pub fn string_slice(s: &Value, start: &Value, end: &Value) -> Value { string_slice_impl(s, start, end) }
pub fn string_substring(s: &Value, start: &Value, end: &Value) -> Value { string_substring_impl(s, start, end) }
pub fn string_split(s: &Value, sep: &Value) -> Value { string_split_impl(s, sep) }
pub fn string_starts_with(s: &Value, search: &Value) -> Value { string_starts_with_impl(s, search) }
pub fn string_ends_with(s: &Value, search: &Value) -> Value { string_ends_with_impl(s, search) }
pub fn string_replace(s: &Value, search: &Value, replacement: &Value) -> Value {
    #[cfg(feature = "regex")]
    if matches!(search, Value::RegExp(_)) {
        return string_replace_regex_or_callback(s, search, replacement);
    }
    string_replace_impl(s, search, replacement)
}
pub fn string_replace_all(s: &Value, search: &Value, replacement: &Value) -> Value { string_replace_all_impl(s, search, replacement) }
pub fn string_char_at(s: &Value, idx: &Value) -> Value { string_char_at_impl(s, idx) }
pub fn string_char_code_at(s: &Value, idx: &Value) -> Value { string_char_code_at_impl(s, idx) }
pub fn string_repeat(s: &Value, count: &Value) -> Value { string_repeat_impl(s, count) }
pub fn string_pad_start(s: &Value, target_len: &Value, pad: &Value) -> Value { string_pad_start_impl(s, target_len, pad) }
pub fn string_pad_end(s: &Value, target_len: &Value, pad: &Value) -> Value { string_pad_end_impl(s, target_len, pad) }

/// Number.prototype.toFixed(digits) - format number with fixed decimal places (0-20)
pub fn number_to_fixed(n: &Value, digits: &Value) -> Value {
    let num = match n {
        Value::Number(x) => *x,
        _ => f64::NAN,
    };
    let d = match digits {
        Value::Number(x) => (*x as i32).clamp(0, 20),
        _ => 0,
    };
    Value::String(format!("{:.*}", d as usize, num).into())
}

/// Operators module for compound assignment operations
pub mod ops {
    use tish_core::Value;

    #[inline]
    pub fn add(left: &Value, right: &Value) -> Result<Value, Box<dyn std::error::Error>> {
        match (left, right) {
            (Value::Number(a), Value::Number(b)) => Ok(Value::Number(a + b)),
            (Value::String(a), Value::String(b)) => {
                let mut s = String::with_capacity(a.len() + b.len());
                s.push_str(a);
                s.push_str(b);
                Ok(Value::String(s.into()))
            }
            (Value::String(a), b) => {
                let b_str = b.to_display_string();
                let mut s = String::with_capacity(a.len() + b_str.len());
                s.push_str(a);
                s.push_str(&b_str);
                Ok(Value::String(s.into()))
            }
            (a, Value::String(b)) => {
                let a_str = a.to_display_string();
                let mut s = String::with_capacity(a_str.len() + b.len());
                s.push_str(&a_str);
                s.push_str(b);
                Ok(Value::String(s.into()))
            }
            _ => Err(format!("Cannot add {:?} and {:?}", left, right).into()),
        }
    }

    #[inline]
    pub fn sub(left: &Value, right: &Value) -> Result<Value, Box<dyn std::error::Error>> {
        match (left, right) {
            (Value::Number(a), Value::Number(b)) => Ok(Value::Number(a - b)),
            _ => Err(format!("Cannot subtract {:?} from {:?}", right, left).into()),
        }
    }

    #[inline]
    pub fn mul(left: &Value, right: &Value) -> Result<Value, Box<dyn std::error::Error>> {
        match (left, right) {
            (Value::Number(a), Value::Number(b)) => Ok(Value::Number(a * b)),
            _ => Err(format!("Cannot multiply {:?} and {:?}", left, right).into()),
        }
    }

    #[inline]
    pub fn div(left: &Value, right: &Value) -> Result<Value, Box<dyn std::error::Error>> {
        match (left, right) {
            (Value::Number(a), Value::Number(b)) => Ok(Value::Number(a / b)),
            _ => Err(format!("Cannot divide {:?} by {:?}", left, right).into()),
        }
    }

    /// Compare two values for <. Supports number vs number and string vs string.
    #[inline]
    pub fn lt(left: &Value, right: &Value) -> Value {
        let b = match (left, right) {
            (Value::Number(a), Value::Number(b)) => a < b,
            (Value::String(a), Value::String(b)) => a.as_ref() < b.as_ref(),
            _ => false,
        };
        Value::Bool(b)
    }

    #[inline]
    pub fn le(left: &Value, right: &Value) -> Value {
        let b = match (left, right) {
            (Value::Number(a), Value::Number(b)) => a <= b,
            (Value::String(a), Value::String(b)) => a.as_ref() <= b.as_ref(),
            _ => false,
        };
        Value::Bool(b)
    }

    #[inline]
    pub fn gt(left: &Value, right: &Value) -> Value {
        let b = match (left, right) {
            (Value::Number(a), Value::Number(b)) => a > b,
            (Value::String(a), Value::String(b)) => a.as_ref() > b.as_ref(),
            _ => false,
        };
        Value::Bool(b)
    }

    #[inline]
    pub fn ge(left: &Value, right: &Value) -> Value {
        let b = match (left, right) {
            (Value::Number(a), Value::Number(b)) => a >= b,
            (Value::String(a), Value::String(b)) => a.as_ref() >= b.as_ref(),
            _ => false,
        };
        Value::Bool(b)
    }

    #[inline]
    pub fn modulo(left: &Value, right: &Value) -> Result<Value, Box<dyn std::error::Error>> {
        match (left, right) {
            (Value::Number(a), Value::Number(b)) => Ok(Value::Number(a % b)),
            _ => Err(format!("Cannot modulo {:?} by {:?}", left, right).into()),
        }
    }
}

use tish_builtins::globals::{
    array_is_array as builtins_array_is_array,
    boolean as builtins_boolean,
    decode_uri as builtins_decode_uri,
    encode_uri as builtins_encode_uri,
    is_finite as builtins_is_finite,
    is_nan as builtins_is_nan,
    object_assign as builtins_object_assign,
    object_entries as builtins_object_entries,
    object_from_entries as builtins_object_from_entries,
    object_keys as builtins_object_keys,
    object_values as builtins_object_values,
    string_from_char_code as builtins_string_from_char_code,
};
use tish_core::{
    json_parse as core_json_parse,
    json_stringify as core_json_stringify,
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

static LOG_LEVEL: OnceLock<LogLevel> = OnceLock::new();

fn get_log_level() -> LogLevel {
    *LOG_LEVEL.get_or_init(|| {
        match std::env::var("TISH_LOG_LEVEL").as_deref() {
            Ok("debug") => LogLevel::Debug,
            Ok("info") => LogLevel::Info,
            Ok("warn") => LogLevel::Warn,
            Ok("error") => LogLevel::Error,
            _ => LogLevel::Log,
        }
    })
}

fn format_args(args: &[Value]) -> String {
    tish_core::format_values_for_console(args, tish_core::use_console_colors())
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
    tish_builtins::globals::parse_int(args)
}

pub fn parse_float(args: &[Value]) -> Value {
    tish_builtins::globals::parse_float(args)
}

pub fn is_finite(args: &[Value]) -> Value {
    builtins_is_finite(args)
}

pub fn is_nan(args: &[Value]) -> Value {
    builtins_is_nan(args)
}

pub fn boolean(args: &[Value]) -> Value {
    builtins_boolean(args)
}

pub fn decode_uri(args: &[Value]) -> Value {
    builtins_decode_uri(args)
}

pub fn encode_uri(args: &[Value]) -> Value {
    builtins_encode_uri(args)
}

// Math functions - use tish_builtins::math
pub use tish_builtins::math::{
    abs as tish_math_abs_impl,
    sqrt as tish_math_sqrt_impl,
    floor as tish_math_floor_impl,
    ceil as tish_math_ceil_impl,
    round as tish_math_round_impl,
    sin as tish_math_sin_impl,
    cos as tish_math_cos_impl,
    tan as tish_math_tan_impl,
    exp as tish_math_exp_impl,
    trunc as tish_math_trunc_impl,
    min as tish_math_min_impl,
    max as tish_math_max_impl,
    pow as tish_math_pow_impl,
    sign as tish_math_sign_impl,
    random as tish_math_random_impl,
};

// Wrapper functions to maintain API (existing callers use math_* naming)
pub fn math_abs(args: &[Value]) -> Value { tish_math_abs_impl(args) }
pub fn math_sqrt(args: &[Value]) -> Value { tish_math_sqrt_impl(args) }
pub fn math_floor(args: &[Value]) -> Value { tish_math_floor_impl(args) }
pub fn math_ceil(args: &[Value]) -> Value { tish_math_ceil_impl(args) }
pub fn math_round(args: &[Value]) -> Value { tish_math_round_impl(args) }
pub fn math_min(args: &[Value]) -> Value { tish_math_min_impl(args) }
pub fn math_max(args: &[Value]) -> Value { tish_math_max_impl(args) }
pub fn math_sin(args: &[Value]) -> Value { tish_math_sin_impl(args) }
pub fn math_cos(args: &[Value]) -> Value { tish_math_cos_impl(args) }
pub fn math_tan(args: &[Value]) -> Value { tish_math_tan_impl(args) }
pub fn math_exp(args: &[Value]) -> Value { tish_math_exp_impl(args) }
pub fn math_trunc(args: &[Value]) -> Value { tish_math_trunc_impl(args) }
pub fn math_pow(args: &[Value]) -> Value { tish_math_pow_impl(args) }
pub fn math_sign(args: &[Value]) -> Value { tish_math_sign_impl(args) }
pub fn math_random(args: &[Value]) -> Value { tish_math_random_impl(args) }

pub fn math_log(args: &[Value]) -> Value {
    let n = extract_num(args.first()).unwrap_or(f64::NAN);
    Value::Number(n.ln())
}

pub fn json_stringify(args: &[Value]) -> Value {
    let v = args.first().cloned().unwrap_or(Value::Null);
    Value::String(core_json_stringify(&v).into())
}

pub fn json_parse(args: &[Value]) -> Value {
    let s = args.first().map(|v| v.to_display_string()).unwrap_or_default();
    core_json_parse(&s).unwrap_or(Value::Null)
}

pub fn date_now(_args: &[Value]) -> Value {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as f64)
        .unwrap_or(0.0);
    Value::Number(now)
}

pub fn array_is_array(args: &[Value]) -> Value {
    builtins_array_is_array(args)
}

pub fn string_from_char_code(args: &[Value]) -> Value {
    builtins_string_from_char_code(args)
}

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

#[cfg(feature = "process")]
pub fn process_exec(args: &[Value]) -> Value {
    use std::process::Command;
    let cmd = args.first().map(|v| v.to_display_string()).unwrap_or_default();
    if cmd.is_empty() {
        return Value::Number(0.0);
    }
    match Command::new("sh").arg("-c").arg(&cmd).status() {
        Ok(status) => Value::Number(status.code().unwrap_or(1) as f64),
        Err(_) => Value::Number(1.0),
    }
}

#[cfg(feature = "fs")]
pub fn read_file(args: &[Value]) -> Value {
    let path = args.first().map(|v| v.to_display_string()).unwrap_or_default();
    match std::fs::read_to_string(&path) {
        Ok(content) => Value::String(content.into()),
        Err(e) => make_error_value(e),
    }
}

#[cfg(feature = "fs")]
pub fn write_file(args: &[Value]) -> Value {
    let path = args.first().map(|v| v.to_display_string()).unwrap_or_default();
    let content = args.get(1).map(|v| v.to_display_string()).unwrap_or_default();
    match std::fs::write(&path, &content) {
        Ok(()) => Value::Bool(true),
        Err(e) => make_error_value(e),
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
        Err(e) => make_error_value(e),
    }
}

#[cfg(feature = "fs")]
pub fn mkdir(args: &[Value]) -> Value {
    let path = args.first().map(|v| v.to_display_string()).unwrap_or_default();
    match std::fs::create_dir_all(&path) {
        Ok(()) => Value::Bool(true),
        Err(e) => make_error_value(e),
    }
}

use std::sync::Arc;

#[inline]
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
        #[cfg(feature = "regex")]
        Value::RegExp(re) => {
            let re = Rc::clone(re);
            if key == "exec" {
                Value::Function(Rc::new(move |args: &[Value]| {
                    let input = args.first().unwrap_or(&Value::Null);
                    regexp_exec(&Value::RegExp(Rc::clone(&re)), input)
                }))
            } else if key == "test" {
                Value::Function(Rc::new(move |args: &[Value]| {
                    let input = args.first().unwrap_or(&Value::Null);
                    regexp_test(&Value::RegExp(Rc::clone(&re)), input)
                }))
            } else {
                Value::Null
            }
        }
        Value::Opaque(o) => o
            .get_method(key)
            .map(Value::Function)
            .unwrap_or(Value::Null),
        _ => Value::Null,
    }
}

#[inline]
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

#[inline]
pub fn set_prop(obj: &Value, key: &str, val: Value) -> Value {
    match obj {
        Value::Object(map) => {
            map.borrow_mut().insert(Arc::from(key), val.clone());
            val
        }
        _ => panic!("Cannot assign property on non-object"),
    }
}

#[inline]
pub fn set_index(obj: &Value, idx: &Value, val: Value) -> Value {
    match obj {
        Value::Array(arr) => {
            let index = match idx {
                Value::Number(n) => *n as usize,
                _ => panic!("Array index must be number"),
            };
            let mut arr_mut = arr.borrow_mut();
            while arr_mut.len() <= index {
                arr_mut.push(Value::Null);
            }
            arr_mut[index] = val.clone();
            val
        }
        Value::Object(map) => {
            let key: Arc<str> = match idx {
                Value::Number(n) => n.to_string().into(),
                Value::String(s) => Arc::clone(s),
                _ => panic!("Object key must be string or number"),
            };
            map.borrow_mut().insert(key, val.clone());
            val
        }
        _ => panic!("Cannot index assign on non-array/object"),
    }
}

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

// Object functions - delegate to tish_builtins::globals
pub fn object_assign(args: &[Value]) -> Value {
    builtins_object_assign(args)
}

pub fn object_keys(args: &[Value]) -> Value {
    builtins_object_keys(args)
}

pub fn object_values(args: &[Value]) -> Value {
    builtins_object_values(args)
}

pub fn object_entries(args: &[Value]) -> Value {
    builtins_object_entries(args)
}

pub fn object_from_entries(args: &[Value]) -> Value {
    builtins_object_from_entries(args)
}

// HTTP Support
#[cfg(feature = "http")]
mod http;

#[cfg(feature = "http")]
mod timers;

#[cfg(feature = "http")]
mod promise;

#[cfg(feature = "http")]
mod native_promise;

#[cfg(feature = "http")]
pub use http::{
    fetch as http_fetch,
    fetch_all as http_fetch_all,
    await_fetch as http_await_fetch,
    await_fetch_all as http_await_fetch_all,
    fetch_async as http_fetch_async,
    fetch_all_async as http_fetch_all_async,
    serve as http_serve,
};

#[cfg(feature = "http")]
pub use timers::{set_timeout as timer_set_timeout, clear_timeout as timer_clear_timeout};

#[cfg(feature = "http")]
pub use promise::promise_object;

#[cfg(feature = "http")]
pub use native_promise::{fetch_async_promise, await_promise};

// RegExp Support
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
    use std::collections::HashMap;

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

            let mut obj: HashMap<std::sync::Arc<str>, Value> = HashMap::new();
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
                
                while let Ok(Some(m)) = re.regex.find_from_pos(input, re.last_index) {
                    matches.push(Value::String(m.as_str().into()));
                    if m.start() == m.end() {
                        re.last_index = m.end() + 1;
                    } else {
                        re.last_index = m.end();
                    }
                    if re.last_index > input.len() { break; }
                }
                
                re.last_index = 0;
                
                if matches.is_empty() { Value::Null } else { Value::Array(Rc::new(RefCell::new(matches))) }
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
fn string_replace_regex_or_callback(s: &Value, search: &Value, replacement: &Value) -> Value {
    let input = match s {
        Value::String(s) => s.as_ref(),
        _ => return s.clone(),
    };

    let Value::RegExp(re) = search else {
        return s.clone();
    };
    let re_guard = re.borrow();

    if let Value::Function(cb) = replacement {
        let limit = if re_guard.flags.global { usize::MAX } else { 1 };
        let mut result = String::new();
        let mut last_end: usize = 0;
        for (count, cap_result) in re_guard.regex.captures_iter(input).enumerate() {
            if count >= limit {
                break;
            }
            let Ok(caps) = cap_result else {
                break;
            };
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

            let repl_val = cb(&args);
            let repl_str = repl_val.to_display_string();
            result.push_str(&input[last_end..byte_start]);
            result.push_str(&repl_str);
            last_end = full.end();
        }

        result.push_str(&input[last_end..]);
        Value::String(result.into())
    } else {
        let replacement_str = replacement.to_display_string();
        if re_guard.flags.global {
            match re_guard.regex.replace_all(input, replacement_str.as_str()) {
                std::borrow::Cow::Borrowed(x) => Value::String(x.into()),
                std::borrow::Cow::Owned(x) => Value::String(x.into()),
            }
        } else {
            match re_guard.regex.replace(input, replacement_str.as_str()) {
                std::borrow::Cow::Borrowed(x) => Value::String(x.into()),
                std::borrow::Cow::Owned(x) => Value::String(x.into()),
            }
        }
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

