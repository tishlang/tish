//! Minimal runtime for Tish compiled output.
//!
//! Re-exports core types from tishlang_core and provides console, Math,
//! and other builtin functions for compiled Tish programs.

use std::fmt;
use std::sync::OnceLock;
use tishlang_builtins::helpers::extract_num;
#[cfg(feature = "fs")]
use tishlang_builtins::helpers::make_error_value;

pub use tishlang_builtins::symbol::symbol_object;
pub use tishlang_core::ObjectMap;
pub use tishlang_core::Value;
pub use tishlang_core::ArcStr;
/// Used by native codegen for `f()` / `obj()` dispatch (`Value::Function` or `__call` on objects).
pub use tishlang_core::value_call;
// Re-export the shared-mutable wrapper so the Rust code emitted by
// `tishlang_compile::codegen` can write `VmRef::new(...)` without needing
// a direct dependency on `tishlang_core` from the generated crate.
pub use tishlang_core::{VmReadGuard, VmRef, VmWriteGuard};

pub use tishlang_builtins::construct::{
    audio_context_constructor_value as tish_audio_context_constructor, construct as tish_construct,
};
pub use tishlang_builtins::typedarrays::{
    float32_array_constructor_value as tish_float32_array_constructor,
    float64_array_constructor_value as tish_float64_array_constructor,
    float64_array_packed,
    int16_array_constructor_value as tish_int16_array_constructor,
    int32_array_constructor_value as tish_int32_array_constructor,
    int8_array_constructor_value as tish_int8_array_constructor,
    uint16_array_constructor_value as tish_uint16_array_constructor,
    uint32_array_constructor_value as tish_uint32_array_constructor,
    uint8_array_constructor_value as tish_uint8_array_constructor,
    uint8_clamped_array_constructor_value as tish_uint8_clamped_array_constructor,
};
pub use tishlang_builtins::date::date_constructor_value as tish_date_constructor;
pub use tishlang_builtins::collections::{
    collection_size, map_constructor_value as tish_map_constructor,
    set_constructor_value as tish_set_constructor,
};

// Re-export array methods from tishlang_builtins
pub use tishlang_builtins::array::{
    concat as array_concat_impl, every as array_every, filter as array_filter, find as array_find,
    find_index as array_find_index, flat as array_flat_impl, flat_map as array_flat_map,
    for_each as array_for_each, includes as array_includes_impl, index_of as array_index_of_impl,
    join as array_join_impl, map as array_map, pop as array_pop, push as array_push_impl,
    reduce as array_reduce, reverse as array_reverse, shift as array_shift,
    shuffle as array_shuffle, slice as array_slice_impl, some as array_some,
    sort_default as array_sort_default, sort_numeric_asc as array_sort_numeric_asc,
    sort_numeric_desc as array_sort_numeric_desc,
    sort_with_comparator as array_sort_with_comparator, splice as array_splice_impl,
    unshift as array_unshift_impl,
};

// Re-export string methods from tishlang_builtins
pub use tishlang_builtins::string::{
    char_at as string_char_at_impl, char_code_at as string_char_code_at_impl,
    ends_with as string_ends_with_impl, escape_html as string_escape_html_impl,
    includes as string_includes_impl, index_of as string_index_of_impl,
    last_index_of as string_last_index_of_impl, pad_end as string_pad_end_impl,
    pad_start as string_pad_start_impl, repeat as string_repeat_impl,
    replace as string_replace_impl, replace_all as string_replace_all_impl,
    slice as string_slice_impl, split as string_split_impl,
    starts_with as string_starts_with_impl, substr as string_substr_impl,
    substring as string_substring_impl,
    to_lower_case as string_to_lower_case, to_upper_case as string_to_upper_case,
    trim as string_trim,
};

// Wrapper functions to maintain API compatibility
pub fn array_push(arr: &Value, args: &[Value]) -> Value {
    array_push_impl(arr, args)
}
pub fn array_unshift(arr: &Value, args: &[Value]) -> Value {
    array_unshift_impl(arr, args)
}
pub fn array_index_of(arr: &Value, search: &Value) -> Value {
    array_index_of_impl(arr, search)
}
pub fn array_includes(arr: &Value, search: &Value, from: &Value) -> Value {
    array_includes_impl(arr, search, Some(from))
}
pub fn array_join(arr: &Value, sep: &Value) -> Value {
    array_join_impl(arr, sep)
}
pub fn array_splice(
    arr: &Value,
    start: &Value,
    delete_count: Option<&Value>,
    items: &[Value],
) -> Value {
    array_splice_impl(arr, start, delete_count, items)
}
pub fn array_slice(arr: &Value, start: &Value, end: &Value) -> Value {
    array_slice_impl(arr, start, end)
}
pub fn array_concat(arr: &Value, args: &[Value]) -> Value {
    array_concat_impl(arr, args)
}
pub fn array_flat(arr: &Value, depth: &Value) -> Value {
    array_flat_impl(arr, depth)
}

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
pub fn string_slice(s: &Value, start: &Value, end: &Value) -> Value {
    string_slice_impl(s, start, end)
}
pub fn string_substring(s: &Value, start: &Value, end: &Value) -> Value {
    string_substring_impl(s, start, end)
}
pub fn string_substr(s: &Value, start: &Value, length: &Value) -> Value {
    string_substr_impl(s, start, length)
}
pub fn string_split(s: &Value, sep: &Value) -> Value {
    // A RegExp separator routes to the regex splitter (matches string_replace's regex handling
    // and the interpreter/VM), so `"a1b2c".split(RegExp("\\d",""))` works on the rust backend.
    #[cfg(feature = "regex")]
    if matches!(sep, Value::RegExp(_)) {
        return string_split_regex(s, sep, None);
    }
    string_split_impl(s, sep)
}
pub fn string_starts_with(s: &Value, search: &Value) -> Value {
    string_starts_with_impl(s, search)
}
pub fn string_ends_with(s: &Value, search: &Value) -> Value {
    string_ends_with_impl(s, search)
}
pub fn string_replace(s: &Value, search: &Value, replacement: &Value) -> Value {
    #[cfg(feature = "regex")]
    if matches!(search, Value::RegExp(_)) {
        return string_replace_regex_or_callback(s, search, replacement);
    }
    string_replace_impl(s, search, replacement)
}
pub fn string_replace_all(s: &Value, search: &Value, replacement: &Value) -> Value {
    string_replace_all_impl(s, search, replacement)
}
pub fn string_char_at(s: &Value, idx: &Value) -> Value {
    string_char_at_impl(s, idx)
}
pub fn string_char_code_at(s: &Value, idx: &Value) -> Value {
    string_char_code_at_impl(s, idx)
}
pub fn string_repeat(s: &Value, count: &Value) -> Value {
    string_repeat_impl(s, count)
}
pub fn string_pad_start(s: &Value, target_len: &Value, pad: &Value) -> Value {
    string_pad_start_impl(s, target_len, pad)
}
pub fn string_pad_end(s: &Value, target_len: &Value, pad: &Value) -> Value {
    string_pad_end_impl(s, target_len, pad)
}
pub fn string_last_index_of(s: &Value, search: &Value, position: &Value) -> Value {
    string_last_index_of_impl(s, search, position)
}

/// Number.prototype.toFixed(digits) - format number with fixed decimal places (0-20)
///
/// Delegates to the single source of truth in `tishlang_builtins::number` so the rust
/// backend, the bytecode VM, and the interpreter stay byte-identical. See
/// `tish/docs/full-backend-parity-plan.md` (Workstream A).
pub fn number_to_fixed(n: &Value, digits: &Value) -> Value {
    tishlang_builtins::number::to_fixed(n, digits)
}

/// Operators module for compound assignment operations
pub mod ops {
    use tishlang_core::Value;

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
                let b_str = b.to_js_string();
                let mut s = String::with_capacity(a.len() + b_str.len());
                s.push_str(a);
                s.push_str(&b_str);
                Ok(Value::String(s.into()))
            }
            (a, Value::String(b)) => {
                let a_str = a.to_js_string();
                let mut s = String::with_capacity(a_str.len() + b.len());
                s.push_str(&a_str);
                s.push_str(b);
                Ok(Value::String(s.into()))
            }
            // Neither operand is a string here ⇒ numeric coercion, matching the VM's `eval_binop`
            // (`as_number().unwrap_or(NaN)`): a null/bool/object operand (e.g. an out-of-bounds array
            // read) coerces to NaN, so `number + null` is NaN — NOT an error that the codegen's
            // `.unwrap_or(Value::Null)` would silently turn into `null` (the old rust-AOT divergence).
            (a, b) => Ok(Value::Number(
                a.as_number().unwrap_or(f64::NAN) + b.as_number().unwrap_or(f64::NAN),
            )),
        }
    }

    #[inline]
    pub fn sub(left: &Value, right: &Value) -> Result<Value, Box<dyn std::error::Error>> {
        match (left, right) {
            (Value::Number(a), Value::Number(b)) => Ok(Value::Number(a - b)),
            // VM-parity numeric coercion (null/non-number → NaN), see `add`.
            (a, b) => Ok(Value::Number(
                a.as_number().unwrap_or(f64::NAN) - b.as_number().unwrap_or(f64::NAN),
            )),
        }
    }

    #[inline]
    pub fn mul(left: &Value, right: &Value) -> Result<Value, Box<dyn std::error::Error>> {
        match (left, right) {
            (Value::Number(a), Value::Number(b)) => Ok(Value::Number(a * b)),
            // VM-parity numeric coercion (null/non-number → NaN), see `add`.
            (a, b) => Ok(Value::Number(
                a.as_number().unwrap_or(f64::NAN) * b.as_number().unwrap_or(f64::NAN),
            )),
        }
    }

    #[inline]
    pub fn div(left: &Value, right: &Value) -> Result<Value, Box<dyn std::error::Error>> {
        match (left, right) {
            (Value::Number(a), Value::Number(b)) => Ok(Value::Number(a / b)),
            // VM-parity numeric coercion (null/non-number → NaN), see `add`.
            (a, b) => Ok(Value::Number(
                a.as_number().unwrap_or(f64::NAN) / b.as_number().unwrap_or(f64::NAN),
            )),
        }
    }

    /// Compare two values for <. Supports number vs number and string vs string.
    #[inline]
    pub fn lt(left: &Value, right: &Value) -> Value {
        let b = match (left, right) {
            (Value::Number(a), Value::Number(b)) => a < b,
            (Value::String(a), Value::String(b)) => a.as_str() < b.as_str(),
            _ => false,
        };
        Value::Bool(b)
    }

    #[inline]
    pub fn le(left: &Value, right: &Value) -> Value {
        let b = match (left, right) {
            (Value::Number(a), Value::Number(b)) => a <= b,
            (Value::String(a), Value::String(b)) => a.as_str() <= b.as_str(),
            _ => false,
        };
        Value::Bool(b)
    }

    #[inline]
    pub fn gt(left: &Value, right: &Value) -> Value {
        let b = match (left, right) {
            (Value::Number(a), Value::Number(b)) => a > b,
            (Value::String(a), Value::String(b)) => a.as_str() > b.as_str(),
            _ => false,
        };
        Value::Bool(b)
    }

    #[inline]
    pub fn ge(left: &Value, right: &Value) -> Value {
        let b = match (left, right) {
            (Value::Number(a), Value::Number(b)) => a >= b,
            (Value::String(a), Value::String(b)) => a.as_str() >= b.as_str(),
            _ => false,
        };
        Value::Bool(b)
    }

    #[inline]
    pub fn modulo(left: &Value, right: &Value) -> Result<Value, Box<dyn std::error::Error>> {
        match (left, right) {
            (Value::Number(a), Value::Number(b)) => Ok(Value::Number(a % b)),
            // VM-parity numeric coercion (null/non-number → NaN), see `add`.
            (a, b) => Ok(Value::Number(
                a.as_number().unwrap_or(f64::NAN) % b.as_number().unwrap_or(f64::NAN),
            )),
        }
    }
}

use tishlang_builtins::globals::{
    array_is_array as builtins_array_is_array, boolean as builtins_boolean,
    decode_uri as builtins_decode_uri, encode_uri as builtins_encode_uri,
    is_finite as builtins_is_finite, is_nan as builtins_is_nan,
    object_assign as builtins_object_assign, object_entries as builtins_object_entries,
    object_from_entries as builtins_object_from_entries, object_keys as builtins_object_keys,
    object_values as builtins_object_values,
    string_convert as builtins_string_convert,
    string_from_char_code as builtins_string_from_char_code,
};
use tishlang_core::{json_parse as core_json_parse, json_stringify as core_json_stringify};

/// Public JSON helpers used by codegen-emitted code (specifically the
/// `_tish_write_json` impls on user-declared `type` aliases). Re-exporting
/// from the runtime keeps the generated source decoupled from
/// `tishlang_core` — generated code only ever names `tishlang_runtime`.
pub mod json {
    pub use tishlang_core::json_stringify_into as stringify_into;
    /// Append the JSON-escaped contents of `s` (without surrounding
    /// quotes) to `buf`. Used by typed-struct serialisers for `String`
    /// fields. Falls through to `tishlang_core::json_stringify_into`'s
    /// internal helper via a `Value::String` round-trip when the inner
    /// helper isn't directly exposed.
    pub fn escape_into(buf: &mut String, s: &str) {
        // Inline the same escape rules as tishlang_core::json::
        // `escape_json_string_into`. Kept locally so we don't widen
        // tishlang_core's public surface unnecessarily.
        let bytes = s.as_bytes();
        let mut start = 0usize;
        for (i, &b) in bytes.iter().enumerate() {
            if b < 0x20 || b == b'"' || b == b'\\' {
                if start < i {
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
}

/// Error type for Tish throw/catch + non-local control flow (used to model `return`/`throw`
/// escaping `try`/`finally` in the Rust backend, which has no native exceptions).
#[derive(Debug, Clone)]
pub enum TishError {
    /// A JS `throw` — catchable by `catch`.
    Throw(Value),
    /// A JS `return value` that must escape an enclosing `try`/`finally` and unwind to the
    /// function boundary (running each `finally` on the way out). Never caught by `catch`.
    Return(Value),
}

impl fmt::Display for TishError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TishError::Throw(v) => write!(f, "{}", v.to_display_string()),
            TishError::Return(v) => write!(f, "return {}", v.to_display_string()),
        }
    }
}

impl std::error::Error for TishError {}

/// Function-boundary unwind: convert a completion that escaped a function body's `Result`-closure
/// back into the function's `Value`. A `return v` yields `v`; an uncaught `throw` panics (matching
/// the behavior of a throw with no enclosing `try`); any other error panics.
pub fn fn_unwind(e: Box<dyn std::error::Error>) -> Value {
    match e.downcast::<TishError>() {
        Ok(te) => match *te {
            TishError::Return(v) => v,
            TishError::Throw(v) => panic!("uncaught throw: {}", v.to_display_string()),
        },
        Err(orig) => panic!("error in native Tish: {:?}", orig),
    }
}

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
    *LOG_LEVEL.get_or_init(|| match std::env::var("TISH_LOG_LEVEL").as_deref() {
        Ok("debug") => LogLevel::Debug,
        Ok("info") => LogLevel::Info,
        Ok("warn") => LogLevel::Warn,
        Ok("error") => LogLevel::Error,
        _ => LogLevel::Log,
    })
}

fn format_args(args: &[Value]) -> String {
    tishlang_core::format_values_for_console(args, tishlang_core::use_console_colors())
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
    tishlang_builtins::globals::parse_int(args)
}

pub fn parse_float(args: &[Value]) -> Value {
    tishlang_builtins::globals::parse_float(args)
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

// Math functions - use tishlang_builtins::math
pub use tishlang_builtins::math::{
    abs as tish_math_abs_impl, ceil as tish_math_ceil_impl, cos as tish_math_cos_impl,
    exp as tish_math_exp_impl, floor as tish_math_floor_impl, max as tish_math_max_impl,
    min as tish_math_min_impl, pow as tish_math_pow_impl, random as tish_math_random_impl,
    round as tish_math_round_impl, sign as tish_math_sign_impl, sin as tish_math_sin_impl,
    imul as tish_math_imul_impl,
    sqrt as tish_math_sqrt_impl, tan as tish_math_tan_impl, trunc as tish_math_trunc_impl,
};

// Wrapper functions to maintain API (existing callers use math_* naming)
pub fn math_abs(args: &[Value]) -> Value {
    tish_math_abs_impl(args)
}
pub fn math_sqrt(args: &[Value]) -> Value {
    tish_math_sqrt_impl(args)
}
pub fn math_floor(args: &[Value]) -> Value {
    tish_math_floor_impl(args)
}
pub fn math_ceil(args: &[Value]) -> Value {
    tish_math_ceil_impl(args)
}
pub fn math_round(args: &[Value]) -> Value {
    tish_math_round_impl(args)
}
pub fn math_min(args: &[Value]) -> Value {
    tish_math_min_impl(args)
}
pub fn math_max(args: &[Value]) -> Value {
    tish_math_max_impl(args)
}
pub fn math_sin(args: &[Value]) -> Value {
    tish_math_sin_impl(args)
}
pub fn math_cos(args: &[Value]) -> Value {
    tish_math_cos_impl(args)
}
pub fn math_tan(args: &[Value]) -> Value {
    tish_math_tan_impl(args)
}
pub fn math_exp(args: &[Value]) -> Value {
    tish_math_exp_impl(args)
}
pub fn math_trunc(args: &[Value]) -> Value {
    tish_math_trunc_impl(args)
}
pub fn math_imul(args: &[Value]) -> Value {
    tish_math_imul_impl(args)
}
pub fn math_pow(args: &[Value]) -> Value {
    tish_math_pow_impl(args)
}
pub fn math_sign(args: &[Value]) -> Value {
    tish_math_sign_impl(args)
}
pub fn math_random(args: &[Value]) -> Value {
    tish_math_random_impl(args)
}

pub fn math_log(args: &[Value]) -> Value {
    let n = extract_num(args.first()).unwrap_or(f64::NAN);
    Value::Number(n.ln())
}

pub fn json_stringify(args: &[Value]) -> Value {
    let v = args.first().cloned().unwrap_or(Value::Null);
    Value::String(core_json_stringify(&v).into())
}

pub fn json_parse(args: &[Value]) -> Value {
    let s = args
        .first()
        .map(|v| v.to_display_string())
        .unwrap_or_default();
    core_json_parse(&s).unwrap_or(Value::Null)
}


pub fn array_is_array(args: &[Value]) -> Value {
    builtins_array_is_array(args)
}

pub fn string_from_char_code(args: &[Value]) -> Value {
    builtins_string_from_char_code(args)
}

/// `String(value)` as a function (JS `ToString`). Wired into the codegen `String`
/// global as `__call` so compiled `String(x)` matches the VM/interp.
pub fn string_convert(args: &[Value]) -> Value {
    builtins_string_convert(args)
}

#[cfg(feature = "process")]
pub fn process_exit(args: &[Value]) -> Value {
    let code = args
        .first()
        .and_then(|v| match v {
            Value::Number(n) => Some(*n as i32),
            _ => None,
        })
        .unwrap_or(0);
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
    let cmd = args
        .first()
        .map(|v| v.to_display_string())
        .unwrap_or_default();
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
    let path = args
        .first()
        .map(|v| v.to_display_string())
        .unwrap_or_default();
    match std::fs::read_to_string(&path) {
        Ok(content) => Value::String(content.into()),
        Err(e) => make_error_value(e),
    }
}

#[cfg(feature = "fs")]
pub fn write_file(args: &[Value]) -> Value {
    let path = args
        .first()
        .map(|v| v.to_display_string())
        .unwrap_or_default();
    let content = args
        .get(1)
        .map(|v| v.to_display_string())
        .unwrap_or_default();
    match std::fs::write(&path, &content) {
        Ok(()) => Value::Bool(true),
        Err(e) => make_error_value(e),
    }
}

#[cfg(feature = "fs")]
pub fn file_exists(args: &[Value]) -> Value {
    let path = args
        .first()
        .map(|v| v.to_display_string())
        .unwrap_or_default();
    Value::Bool(std::path::Path::new(&path).exists())
}

#[cfg(feature = "fs")]
pub fn is_dir(args: &[Value]) -> Value {
    let path = args
        .first()
        .map(|v| v.to_display_string())
        .unwrap_or_default();
    Value::Bool(std::path::Path::new(&path).is_dir())
}

#[cfg(feature = "fs")]
pub fn read_dir(args: &[Value]) -> Value {
    let path = args
        .first()
        .map(|v| v.to_display_string())
        .unwrap_or_else(|| ".".to_string());
    match std::fs::read_dir(&path) {
        Ok(entries) => {
            let files: Vec<Value> = entries
                .filter_map(|e| e.ok())
                .map(|e| Value::String(e.file_name().to_string_lossy().into()))
                .collect();
            Value::Array(VmRef::new(files))
        }
        Err(e) => make_error_value(e),
    }
}

#[cfg(feature = "fs")]
pub fn mkdir(args: &[Value]) -> Value {
    let path = args
        .first()
        .map(|v| v.to_display_string())
        .unwrap_or_default();
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
            // `Set`/`Map` instances expose a computed `.size` (the backing store has no real
            // `size` key); `collection_size` returns `None` for any other object.
            if key == "size" {
                if let Some(n) = collection_size(obj) {
                    return Value::Number(n);
                }
            }
            // The map's key type is `Arc<str>`, which implements
            // `Borrow<str>` — so we can look up with a borrowed `&str`
            // directly. Previously we allocated a fresh `Arc<str>` on
            // every call (one heap alloc per `r.field` read in tight
            // handler loops); this version is alloc-free on the hit path.
            map.borrow().strings.get(key).cloned().unwrap_or(Value::Null)
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
        // Packed `Float64Array` (`TISH_PACKED_ARRAYS`): `.length` and numeric-key reads, mirroring
        // the boxed `Array` arm. Methods (`reduce`/`map`/…) materialise via `as_boxed_array`.
        Value::NumberArray(arr) => {
            if key == "length" {
                Value::Number(arr.borrow().len() as f64)
            } else if let Ok(idx) = key.parse::<usize>() {
                arr.borrow().get(idx).copied().map(Value::Number).unwrap_or(Value::Null)
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
        Value::RegExp(re) => match key {
            "exec" => {
                let rc = re.clone();
                Value::native(move |args: &[Value]| {
                    let input = args.first().unwrap_or(&Value::Null);
                    regexp_exec(&Value::RegExp(rc.clone()), input)
                })
            }
            "test" => {
                let rc = re.clone();
                Value::native(move |args: &[Value]| {
                    let input = args.first().unwrap_or(&Value::Null);
                    regexp_test(&Value::RegExp(rc.clone()), input)
                })
            }
            // Properties — mirror the interpreter + bytecode VM so all backends agree.
            "source" => Value::String(re.borrow().source.clone().into()),
            "flags" => Value::String(re.borrow().flags_string().into()),
            "lastIndex" => Value::Number(re.borrow().last_index as f64),
            "global" => Value::Bool(re.borrow().flags.global),
            "ignoreCase" => Value::Bool(re.borrow().flags.ignore_case),
            "multiline" => Value::Bool(re.borrow().flags.multiline),
            "dotAll" => Value::Bool(re.borrow().flags.dot_all),
            "unicode" => Value::Bool(re.borrow().flags.unicode),
            "sticky" => Value::Bool(re.borrow().flags.sticky),
            _ => Value::Null,
        },
        Value::Opaque(o) => o
            .get_method(key)
            .map(Value::Function)
            .unwrap_or(Value::Null),
        // Promise instance methods (`.then`/`.catch`), bound to this promise. Returning a
        // callable here makes the rust backend match the VM family (interp/vm/cranelift/wasi),
        // which expose these via `GetMember`. Both `p.then(cb)` (member) and `p["catch"](cb)`
        // (index, used because `catch` is reserved) route through here / `get_index`.
        #[cfg(any(feature = "http", feature = "promise"))]
        Value::Promise(p) => match key {
            "then" => {
                let pc = p.clone();
                Value::native(move |args: &[Value]| promise_instance_then(&pc, args))
            }
            "catch" => {
                let pc = p.clone();
                Value::native(move |args: &[Value]| promise_instance_catch(&pc, args))
            }
            _ => Value::Null,
        },
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
        // Packed `Float64Array` indexing (`TISH_PACKED_ARRAYS`); mirrors the boxed `Array` arm.
        Value::NumberArray(arr) => {
            let idx = match index {
                Value::Number(n) => *n as usize,
                _ => return Value::Null,
            };
            arr.borrow().get(idx).copied().map(Value::Number).unwrap_or(Value::Null)
        }
        Value::Object(_) => tishlang_core::object_get(obj, index).unwrap_or(Value::Null),
        // `promise["then"|"catch"]` — string-keyed access mirrors `get_prop` (bracket form
        // is required for `catch`, a reserved word). Keeps the rust backend on par with the VM.
        #[cfg(any(feature = "http", feature = "promise"))]
        Value::Promise(_) => match index {
            Value::String(k) => get_prop(obj, k.as_str()),
            _ => Value::Null,
        },
        _ => Value::Null,
    }
}

#[inline]
pub fn set_prop(obj: &Value, key: &str, val: Value) -> Value {
    match obj {
        Value::Object(map) => {
            // Try the in-place update path first: if the key already
            // exists we re-use the existing `Arc<str>` and skip the
            // alloc. Only newly-inserted keys pay for `Arc::from(key)`.
            let mut m = map.borrow_mut();
            if let Some(slot) = m.strings.get_mut(key) {
                *slot = val.clone();
            } else {
                m.strings.insert(Arc::from(key), val.clone());
            }
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
        // Packed `Float64Array` write (`TISH_PACKED_ARRAYS`). On the native path a `NumberArray` is
        // always a `Float64Array`, so storing the f64 (non-numeric → `NaN`) is the correct view
        // semantics — and unlike the boxed v1 backing, it actually coerces the write to the element
        // type. Out-of-range index zero-fills, matching the boxed grow-with-Null behaviour.
        Value::NumberArray(arr) => {
            let index = match idx {
                Value::Number(n) => *n as usize,
                _ => panic!("Array index must be number"),
            };
            let mut arr_mut = arr.borrow_mut();
            while arr_mut.len() <= index {
                arr_mut.push(0.0);
            }
            arr_mut[index] = val.as_number().unwrap_or(f64::NAN);
            val
        }
        Value::Object(_) => {
            tishlang_core::object_set(obj, idx, val.clone()).expect("object set");
            val
        }
        _ => panic!("Cannot index assign on non-array/object"),
    }
}

pub fn in_operator(key: &Value, obj: &Value) -> Value {
    match obj {
        Value::Object(_) => Value::Bool(tishlang_core::object_has(obj, key)),
        Value::Array(arr) => {
            let key_str: Arc<str> = match key {
                Value::String(s) => Arc::from(s.as_str()),
                Value::Number(n) => n.to_string().into(),
                _ => return Value::Bool(false),
            };
            let result = key_str.as_ref() == "length"
                || key_str
                    .parse::<usize>()
                    .ok()
                    .map(|i| i < arr.borrow().len())
                    .unwrap_or(false);
            Value::Bool(result)
        }
        // Packed `Float64Array` (`TISH_PACKED_ARRAYS`); same key set as the boxed `Array` arm.
        Value::NumberArray(arr) => {
            let key_str: Arc<str> = match key {
                Value::String(s) => Arc::from(s.as_str()),
                Value::Number(n) => n.to_string().into(),
                _ => return Value::Bool(false),
            };
            let result = key_str.as_ref() == "length"
                || key_str
                    .parse::<usize>()
                    .ok()
                    .map(|i| i < arr.borrow().len())
                    .unwrap_or(false);
            Value::Bool(result)
        }
        _ => Value::Bool(false),
    }
}

// Object functions - delegate to tishlang_builtins::globals
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
mod promise_io;

#[cfg(feature = "http")]
mod http;

#[cfg(feature = "http")]
mod http_prefork;

#[cfg(feature = "http-io-uring")]
mod http_io_uring;

#[cfg(feature = "http-hyper")]
mod http_hyper;

#[cfg(feature = "http")]
mod http_fetch;

mod timers;

#[cfg(any(feature = "http", feature = "promise"))]
mod promise;

#[cfg(feature = "http")]
mod native_promise;

#[cfg(feature = "ws")]
mod ws;

#[cfg(feature = "ws")]
pub use ws::{
    web_socket_client, web_socket_server_accept, web_socket_server_construct,
    web_socket_server_listen, ws_broadcast_native, ws_send_native,
};

#[cfg(feature = "http")]
pub use http::{
    await_fetch as http_await_fetch, await_fetch_all as http_await_fetch_all,
    register_static_route,
};

// `serve` is the user-facing entry point for Tish's HTTP server. By default
// it uses the tiny_http + SO_REUSEPORT path in `http.rs`. When compiled with
// `--features http-hyper` and the `TISH_HTTP_BACKEND=hyper` env var is set
// at runtime, it dispatches to the hyper backend in `http_hyper.rs`.
//
// The env-var switch (rather than a cargo feature switch) means one built
// binary can toggle backends for A/B benchmarking and production rollout
// without rebuilding. When `http-hyper` is not compiled in, the switch is a
// no-op and the tiny_http path is used unconditionally.
#[cfg(feature = "http")]
pub fn http_serve<F>(args: &[tishlang_core::Value], handler: F) -> tishlang_core::Value
where
    F: Fn(&[tishlang_core::Value]) -> tishlang_core::Value + Send + Sync + 'static,
{
    #[cfg(feature = "http-hyper")]
    {
        if http_hyper::is_enabled_via_env() {
            return http_hyper::serve(args, handler);
        }
    }
    http::serve(args, handler)
}

/// `serve(port, { onWorker: (workerId) => handler })` — the object form of
/// `serve`. Picks up `onWorker`, invokes it once per HTTP accept thread to
/// build that thread's handler, then enters the normal parallel accept
/// loop. See [`http::serve_per_worker`] for the full doc.
///
/// This is broadly useful for any Tish app that wants per-worker state —
/// DB connection pools, in-process caches, counters, etc. — without a
/// global `RwLock` or forcing everything through the single-thread
/// dispatcher. It also plays nicely with prefork: `onWorker` sees global
/// worker ids across processes so logs and sharded state are easy to key.
#[cfg(feature = "http")]
pub fn http_serve_per_worker(
    args: &[tishlang_core::Value],
    factory_value: tishlang_core::Value,
) -> tishlang_core::Value {
    use tishlang_core::Value;
    // factory_value should be Value::Function (passed down by codegen after
    // extracting `onWorker` from the options object).
    let Value::Function(factory) = factory_value else {
        eprintln!("[tish http] serve: onWorker must be a function (id) => handler");
        return Value::Null;
    };
    let factory: tishlang_core::NativeFn = factory;
    http::serve_per_worker(args, move |worker_id| {
        let handler_val = factory.call(&[Value::Number(worker_id as f64)]);
        match handler_val {
            Value::Function(f) => f,
            _ => panic!(
                "onWorker returned {:?} for worker {}; must return a function",
                handler_val, worker_id
            ),
        }
    })
}

pub use timers::{
    clear_interval as timer_clear_interval, clear_timeout as timer_clear_timeout, drain_timers,
    set_interval as timer_set_interval, set_timeout as timer_set_timeout,
};

#[cfg(any(feature = "http", feature = "promise"))]
pub use promise::{
    await_promise, await_promise_throw, promise_instance_catch, promise_instance_then,
    promise_object, promise_spawn as promise_spawn_value,
};

#[cfg(feature = "http")]
pub use native_promise::{fetch_all_promise, fetch_async_promise, fetch_promise};

// RegExp Support
#[cfg(feature = "regex")]
pub use tishlang_core::{RegExpFlags, TishRegExp};

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
        Ok(re) => Value::RegExp(VmRef::new(re)),
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
fn regexp_exec_impl(re: &mut tishlang_core::TishRegExp, input: &str) -> Value {
    use tishlang_core::ObjectMap;

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

            let mut obj: ObjectMap = ObjectMap::default();
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

            Value::object(obj)
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
        _ => return Value::Array(VmRef::new(vec![s.clone()])),
    };

    let max = limit.unwrap_or(usize::MAX);
    if max == 0 {
        return Value::Array(VmRef::new(Vec::new()));
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

            Value::Array(VmRef::new(result))
        }
        Value::String(sep) => {
            let parts: Vec<Value> = input
                .splitn(max, sep.as_str())
                .map(|s| Value::String(s.into()))
                .collect();
            Value::Array(VmRef::new(parts))
        }
        _ => Value::Array(VmRef::new(vec![Value::String(input.into())])),
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
                    if re.last_index > input.len() {
                        break;
                    }
                }

                re.last_index = 0;

                if matches.is_empty() {
                    Value::Null
                } else {
                    Value::Array(VmRef::new(matches))
                }
            } else {
                regexp_exec_impl(&mut re, input)
            }
        }
        Value::String(pattern) => match tishlang_core::TishRegExp::new(pattern, "") {
            Ok(mut re) => regexp_exec_impl(&mut re, input),
            Err(_) => Value::Null,
        },
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

            let repl_val = cb.call(&args);
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
        Value::String(pattern) => match tishlang_core::TishRegExp::new(pattern, "") {
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
