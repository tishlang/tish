//! no_std runtime facade for Tish compiled output on the Game Boy Advance.
//!
//! The `Gba` emit mode generates a crate that depends on this one under the name
//! `tishlang_runtime` (via a Cargo `package =` rename), so every emitted
//! `tishlang_runtime::…` path resolves here. It re-exports the portable prelude
//! surface from `tishlang_core` / `tishlang_builtins` and adds the GBA runtime
//! entry points in [`gba`].
#![no_std]

extern crate alloc;

use alloc::boxed::Box;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

// ── Core value vocabulary (single source of truth in tishlang_core) ──────────
pub use tishlang_core::{
    js_number_to_string_into, to_int32, to_int32_value, to_number_value, to_uint32,
    to_uint32_value, ArcStr, NumArrayBacking, ObjectData, ObjectMap, PropMap, Value, VmReadGuard,
    VmRef, VmWriteGuard,
};
/// `Arc` on GBA is `Rc` (single-core). Emitted code writes `Arc::from(..)`.
pub use tishlang_core::Arc;
/// Fixed-point scalar the typed-numeric path lowers `fixed` to (agb `Num<i32,8>`).
pub type Fixed = agb::fixnum::Num<i32, 8>;

// ── Pending-throw + recursion-guard plumbing (shared slot in tishlang_core) ──
pub use tishlang_core::{
    has_pending_throw, set_pending_throw, stack_overflow_error, take_pending_throw, CallDepthGuard,
};

/// Enter a boxed user-fn call frame or trip the recursion guard. On GBA there is
/// no stack-pressure probe (no `std`), so this is just the counted-depth guard.
#[inline]
pub fn enter_call_guarded() -> Option<CallDepthGuard> {
    tishlang_core::enter_call_guarded()
}

// ── Error type for throw/return non-local control flow ───────────────────────
/// Mirrors `tishlang_runtime::TishError` (the host runtime defines its own; kept
/// in lock-step). `run()` in generated code returns `Result<(), Box<dyn Error>>`.
#[derive(Debug, Clone)]
pub enum TishError {
    Throw(Value),
    Return(Value),
}

impl core::fmt::Display for TishError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            TishError::Throw(v) => write!(f, "{}", v.to_display_string()),
            TishError::Return(v) => write!(f, "return {}", v.to_display_string()),
        }
    }
}

impl core::error::Error for TishError {}

/// Convert a boxed error thrown out of `run()` back into a `Value` (throw/return payload).
pub fn fn_unwind(e: Box<dyn core::error::Error>) -> Value {
    if let Some(te) = e.downcast_ref::<TishError>() {
        match te {
            TishError::Throw(v) | TishError::Return(v) => return v.clone(),
        }
    }
    Value::String(e.to_string().into())
}

// ── Value call / concat / read helpers used by native codegen ────────────────
/// `f()` / `obj()` dispatch, suppressed while a throw is unwinding (#381).
pub fn value_call(callee: &Value, args: &[Value]) -> Value {
    if has_pending_throw() {
        return Value::Null;
    }
    tishlang_core::value_call(callee, args)
}

/// Append `v`'s JS string-concat form to `buf` (no throwaway `String`).
#[inline]
pub fn push_value_str(buf: &mut String, v: &Value) {
    match v {
        Value::String(s) => buf.push_str(s),
        Value::Number(n) => js_number_to_string_into(buf, *n),
        Value::Bool(b) => buf.push_str(if *b { "true" } else { "false" }),
        Value::Null => buf.push_str("null"),
        other => buf.push_str(&other.to_js_string()),
    }
}

/// Read a captured `VmRef` cell, releasing the guard before returning (#218).
#[inline]
pub fn vm_read<T: Clone>(cell: &VmRef<T>) -> T {
    (*cell.borrow()).clone()
}

/// `for…of` iterable normalization for the native backend.
pub fn normalize_for_of(v: Value) -> Value {
    if let Value::NumberArray(arr) = &v {
        let items: Vec<Value> = arr.borrow().to_values();
        return Value::Array(VmRef::new(items));
    }
    match tishlang_core::drain_iterator(&v) {
        Some(items) => Value::Array(VmRef::new(items)),
        None => v,
    }
}

/// JS `in` operator (`key in obj`).
pub fn in_operator(key: &Value, obj: &Value) -> Value {
    match obj {
        Value::Object(_) => Value::Bool(tishlang_core::object_has(obj, key)),
        Value::Array(arr) => Value::Bool(array_in(key, arr.borrow().len())),
        Value::NumberArray(arr) => Value::Bool(array_in(key, arr.borrow().len())),
        _ => Value::Bool(false),
    }
}

fn array_in(key: &Value, len: usize) -> bool {
    let key_str: Arc<str> = match key {
        Value::String(s) => Arc::from(s.as_str()),
        Value::Number(n) => n.to_string().into(),
        _ => return false,
    };
    key_str.as_ref() == "length"
        || key_str.parse::<usize>().ok().map(|i| i < len).unwrap_or(false)
}

// ── Member / index read + write (ported from the host runtime; the regex/promise
//    arms are feature-gated off on GBA). Method dispatch (`arr.map(f)`) is emitted
//    as direct builtin calls by codegen, not through these. ──────────────────────
use tishlang_builtins::collections::collection_size;
use tishlang_builtins::helpers::extract_num;

pub fn get_prop(obj: &Value, key: impl AsRef<str>) -> Value {
    let key = key.as_ref();
    match obj {
        Value::Object(map) => {
            if key == "size" {
                if let Some(n) = collection_size(obj) {
                    return Value::Number(n);
                }
            }
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
        Value::NumberArray(arr) => {
            if key == "length" {
                Value::Number(arr.borrow().len() as f64)
            } else if let Ok(idx) = key.parse::<usize>() {
                arr.borrow()
                    .get(idx)
                    .map(|v| match v {
                        Value::Number(n) if n.is_nan() => Value::Null,
                        other => other,
                    })
                    .unwrap_or(Value::Null)
            } else {
                Value::Null
            }
        }
        Value::String(s) => {
            if key == "length" {
                Value::Number(tishlang_builtins::string::char_count(s) as f64)
            } else {
                Value::Null
            }
        }
        Value::Opaque(o) => o.get_method(key).map(Value::Function).unwrap_or(Value::Null),
        Value::Null => {
            tishlang_core::set_pending_throw(tishlang_core::cannot_read_property_error(key));
            Value::Null
        }
        _ => Value::Null,
    }
}

pub fn get_index(obj: &Value, index: &Value) -> Value {
    match obj {
        Value::Array(arr) => {
            let idx = match index {
                Value::Number(n) => *n as usize,
                Value::String(s) => match tishlang_core::str_to_array_index(s) {
                    Some(i) => i,
                    None => return Value::Null,
                },
                _ => return Value::Null,
            };
            arr.borrow().get(idx).cloned().unwrap_or(Value::Null)
        }
        Value::NumberArray(arr) => {
            let idx = match index {
                Value::Number(n) => *n as usize,
                Value::String(s) => match tishlang_core::str_to_array_index(s) {
                    Some(i) => i,
                    None => return Value::Null,
                },
                _ => return Value::Null,
            };
            arr.borrow()
                .get(idx)
                .map(|v| match v {
                    Value::Number(n) if n.is_nan() => Value::Null,
                    other => other,
                })
                .unwrap_or(Value::Null)
        }
        Value::String(s) => match index {
            Value::Number(n) if *n >= 0.0 && n.fract() == 0.0 => {
                tishlang_builtins::string::nth_char(s, *n as usize)
                    .map(|c| Value::String(c.to_string().into()))
                    .unwrap_or(Value::Null)
            }
            _ => Value::Null,
        },
        Value::Object(_) => tishlang_core::object_get(obj, index).unwrap_or(Value::Null),
        Value::Null => {
            let key = match index {
                Value::String(s) => s.to_string(),
                other => other.to_js_string(),
            };
            tishlang_core::set_pending_throw(tishlang_core::cannot_read_property_error(&key));
            Value::Null
        }
        _ => Value::Null,
    }
}

pub fn delete_property(obj: &Value, key: &Value) -> Value {
    match obj {
        Value::Object(m) => {
            let key_s = match key {
                Value::String(s) => s.to_string(),
                other => other.to_js_string(),
            };
            m.borrow_mut().strings.remove(key_s.as_str());
        }
        Value::Array(arr) => {
            if let Value::Number(n) = key {
                let n = *n;
                if n >= 0.0 && n.fract() == 0.0 {
                    let i = n as usize;
                    let mut a = arr.borrow_mut();
                    if i < a.len() {
                        a[i] = Value::Null;
                    }
                }
            }
        }
        _ => {}
    }
    Value::Bool(true)
}

/// A valid JS array index: a non-negative integer `< 2^32-1`. Anything else (negative,
/// fractional, NaN, or too large) is `None` — on GBA we treat such an assignment as a no-op
/// rather than densifying/aborting (JS would set a named property, which a `Vec`-backed array
/// can't hold anyway).
fn array_index(n: f64) -> Option<usize> {
    if n >= 0.0 && n.fract() == 0.0 && n <= 4_294_967_294.0 {
        Some(n as usize)
    } else {
        None
    }
}

/// Grow `v` so `index` is in range, filling with `fill`. Instead of aborting when the
/// allocation can't be satisfied (a real risk on the GBA's 256KB EWRAM for a large sparse
/// index), it raises a CATCHABLE tish error and returns `false` so the caller bails out.
fn grow_or_throw<T: Clone>(v: &mut alloc::vec::Vec<T>, index: usize, fill: T) -> bool {
    if index >= v.len() {
        let extra = index - v.len() + 1;
        if v.try_reserve(extra).is_err() {
            tishlang_core::set_pending_throw(tishlang_core::type_error(format!(
                "array index {index} too large to allocate"
            )));
            return false;
        }
        v.resize(index + 1, fill);
    }
    true
}

pub fn set_prop(obj: &Value, key: &str, val: Value) -> Value {
    match obj {
        Value::Object(map) => {
            let mut m = map.borrow_mut();
            if m.frozen {
                tishlang_core::set_pending_throw(tishlang_core::type_error(format!(
                    "Cannot assign to read only property '{key}' of a frozen object"
                )));
                return val;
            }
            if let Some(slot) = m.strings.get_mut(key) {
                *slot = val.clone();
            } else {
                m.strings.insert(Arc::from(key), val.clone());
            }
            val
        }
        Value::Array(arr) if key == "length" => {
            let n = extract_num(Some(&val)).unwrap_or(f64::NAN);
            if n.is_nan() || n < 0.0 || n.fract() != 0.0 || n > 4_294_967_295.0 {
                tishlang_core::set_pending_throw(tishlang_core::type_error("Invalid array length"));
                return val;
            }
            let len = n as usize;
            let mut arr_mut = arr.borrow_mut();
            if len > arr_mut.len() {
                // grow_or_throw indexes to len-1 (so len is in range); catchable on OOM.
                let _ = grow_or_throw(&mut arr_mut, len - 1, Value::Null);
            } else {
                arr_mut.truncate(len);
            }
            val
        }
        _ => {
            tishlang_core::set_pending_throw(tishlang_core::type_error("Cannot assign property on a non-object"));
            val
        }
    }
}

pub fn set_index(obj: &Value, idx: &Value, val: Value) -> Value {
    // Resolve the array index, or `None` for a non-index key (negative/fractional/huge number,
    // non-numeric string, or non-string/number key). A `None` on an array is a no-op here
    // rather than an abort: JS would set a named property, which a `Vec`-backed array can't hold.
    let arr_index = |idx: &Value| -> Option<usize> {
        match idx {
            Value::Number(n) => array_index(*n),
            Value::String(s) => tishlang_core::str_to_array_index(s),
            _ => None,
        }
    };
    match obj {
        Value::Array(arr) => {
            let Some(index) = arr_index(idx) else {
                return val;
            };
            let mut arr_mut = arr.borrow_mut();
            if !grow_or_throw(&mut arr_mut, index, Value::Null) {
                return val;
            }
            arr_mut[index] = val.clone();
            val
        }
        Value::NumberArray(arr) => {
            let Some(index) = arr_index(idx) else {
                return val;
            };
            let mut b = arr.borrow_mut();
            match (b.as_packed_mut(), val.as_number()) {
                (Some(nums), Some(n)) => {
                    if grow_or_throw(nums, index, f64::NAN) {
                        nums[index] = n;
                    }
                }
                _ => {
                    let boxed = b.deopt();
                    if grow_or_throw(boxed, index, Value::Null) {
                        boxed[index] = val.clone();
                    }
                }
            }
            val
        }
        Value::Object(map) => {
            if map.borrow().frozen {
                tishlang_core::set_pending_throw(tishlang_core::type_error(format!(
                    "Cannot assign to read only property '{}' of a frozen object",
                    idx.to_display_string()
                )));
                return val;
            }
            // object_set only accepts string/number/symbol keys; on any other key raise a
            // CATCHABLE error rather than aborting (`.expect`) the console.
            if tishlang_core::object_set(obj, idx, val.clone()).is_err() {
                tishlang_core::set_pending_throw(tishlang_core::type_error(format!(
                    "cannot use {} as an object key",
                    idx.to_display_string()
                )));
            }
            val
        }
        _ => {
            tishlang_core::set_pending_throw(tishlang_core::type_error("Cannot index-assign on a non-array/object"));
            val
        }
    }
}

// ── console.* → mGBA debug log (agb::println!) ───────────────────────────────
fn console_line(args: &[Value]) -> String {
    tishlang_core::format_values_for_console(args, false)
}
pub fn console_log(args: &[Value]) {
    agb::println!("{}", console_line(args));
}
pub fn console_info(args: &[Value]) {
    agb::println!("{}", console_line(args));
}
pub fn console_debug(args: &[Value]) {
    agb::println!("{}", console_line(args));
}
pub fn console_warn(args: &[Value]) {
    agb::println!("{}", console_line(args));
}
pub fn console_error(args: &[Value]) {
    agb::println!("{}", console_line(args));
}

// ── JSON (native-ABI wrappers over the core `&str`→Result / `&Value`→String fns) ──
pub fn json_parse(args: &[Value]) -> Value {
    let s = args.first().map(|v| v.to_display_string()).unwrap_or_default();
    tishlang_core::json_parse(&s).unwrap_or(Value::Null)
}
pub fn json_stringify(args: &[Value]) -> Value {
    let v = args.first().cloned().unwrap_or(Value::Null);
    Value::String(tishlang_core::json_stringify(&v).into())
}

/// Build an `ObjectMap` from an array of key/value pairs. Generated object literals
/// emit `ObjectMap::from([...])`; hashbrown's `From<[_; N]>` isn't available with a
/// custom hasher, so the Gba post-pass rewrites those calls to this.
pub fn object_map_from<const N: usize>(items: [(Arc<str>, Value); N]) -> ObjectMap {
    let mut m = ObjectMap::default();
    for (k, v) in items {
        m.insert(k, v);
    }
    m
}

/// f64 transcendentals for generated inline math (`Math.sqrt(x)` const-folds to
/// `x.sqrt()`); re-exported so generated code can `use tishlang_runtime::FloatExt`.
pub use tishlang_core::FloatExt;

/// Single-core interior-mutable static helper, re-exported for binding crates
/// (`tish-agb`) that keep hardware context in a `static`.
pub use tishlang_core::SingleCore;

/// Value arithmetic / comparison operators for generated `+`,`-`,… (ported from the
/// host runtime's `ops`; pure `Value` math, no_std-safe).
pub mod ops {
    use crate::Value;
    use alloc::string::String;

    #[inline]
    pub fn add(left: &Value, right: &Value) -> Value {
        match (left, right) {
            (Value::Number(a), Value::Number(b)) => Value::Number(a + b),
            (Value::String(a), Value::String(b)) => {
                let mut s = String::with_capacity(a.len() + b.len());
                s.push_str(a);
                s.push_str(b);
                Value::String(s.into())
            }
            (Value::String(a), b) => {
                let b_str = b.to_js_string();
                let mut s = String::with_capacity(a.len() + b_str.len());
                s.push_str(a);
                s.push_str(&b_str);
                Value::String(s.into())
            }
            (a, Value::String(b)) => {
                let a_str = a.to_js_string();
                let mut s = String::with_capacity(a_str.len() + b.len());
                s.push_str(&a_str);
                s.push_str(b);
                Value::String(s.into())
            }
            (a, b) => Value::Number(
                a.as_number().unwrap_or(f64::NAN) + b.as_number().unwrap_or(f64::NAN),
            ),
        }
    }

    macro_rules! num_op {
        ($name:ident, $op:tt) => {
            #[inline]
            pub fn $name(left: &Value, right: &Value) -> Value {
                match (left, right) {
                    (Value::Number(a), Value::Number(b)) => Value::Number(a $op b),
                    (a, b) => Value::Number(
                        a.as_number().unwrap_or(f64::NAN) $op b.as_number().unwrap_or(f64::NAN),
                    ),
                }
            }
        };
    }
    num_op!(sub, -);
    num_op!(mul, *);
    num_op!(div, /);
    num_op!(modulo, %);

    macro_rules! cmp_op {
        ($name:ident, $op:tt) => {
            #[inline]
            pub fn $name(left: &Value, right: &Value) -> Value {
                let b = match (left, right) {
                    (Value::Number(a), Value::Number(b)) => a $op b,
                    (Value::String(a), Value::String(b)) => a.as_str() $op b.as_str(),
                    _ => false,
                };
                Value::Bool(b)
            }
        };
    }
    cmp_op!(lt, <);
    cmp_op!(le, <=);
    cmp_op!(gt, >);
    cmp_op!(ge, >=);
}

// ── Globals / object / number / string / uri (all live in builtins::globals) ─
pub use tishlang_builtins::globals::{
    array_from, array_is_array, array_of, boolean, decode_uri, decode_uri_component, encode_uri,
    encode_uri_component, is_finite, is_nan, number_convert, number_is_finite, number_is_integer,
    number_is_nan, number_is_safe_integer, object_assign, object_entries, object_freeze,
    object_from_entries, object_has_own, object_is, object_is_frozen, object_keys, object_values,
    parse_float, parse_int, string_convert, string_from_char_code, structured_clone,
};
pub use tishlang_builtins::string::escape_html as string_escape_html_impl;

// ── Constructors, collections, typed arrays, symbol (re-export as prelude names) ──
pub use tishlang_builtins::construct::{
    array_construct, audio_context_constructor_value as tish_audio_context_constructor,
    construct as tish_construct, error_constructor_value as tish_error_constructor,
};
pub use tishlang_builtins::date::date_constructor_value as tish_date_constructor;
pub use tishlang_builtins::collections::{
    map_constructor_value as tish_map_constructor, map_get, map_has, map_set, map_values,
    set_constructor_value as tish_set_constructor,
};
pub use tishlang_builtins::symbol::symbol_object;
pub use tishlang_builtins::typedarrays::{
    float32_array_constructor_value as tish_float32_array_constructor,
    float64_array_constructor_value as tish_float64_array_constructor,
    int16_array_constructor_value as tish_int16_array_constructor,
    int32_array_constructor_value as tish_int32_array_constructor,
    int8_array_constructor_value as tish_int8_array_constructor,
    uint16_array_constructor_value as tish_uint16_array_constructor,
    uint32_array_constructor_value as tish_uint32_array_constructor,
    uint8_array_constructor_value as tish_uint8_array_constructor,
    uint8_clamped_array_constructor_value as tish_uint8_clamped_array_constructor,
};

// ── Math (thin wrappers preserving the `math_*` prelude naming) ──────────────
macro_rules! math_fwd {
    ($($name:ident => $path:path),* $(,)?) => {
        $(
            #[inline]
            pub fn $name(args: &[Value]) -> Value { $path(args) }
        )*
    };
}
math_fwd! {
    math_abs => tishlang_builtins::math::abs,
    math_ceil => tishlang_builtins::math::ceil,
    math_floor => tishlang_builtins::math::floor,
    math_round => tishlang_builtins::math::round,
    math_sqrt => tishlang_builtins::math::sqrt,
    math_sin => tishlang_builtins::math::sin,
    math_cos => tishlang_builtins::math::cos,
    math_tan => tishlang_builtins::math::tan,
    math_asin => tishlang_builtins::math::asin,
    math_acos => tishlang_builtins::math::acos,
    math_atan => tishlang_builtins::math::atan,
    math_atan2 => tishlang_builtins::math::atan2,
    math_log => tishlang_builtins::math::log,
    math_log2 => tishlang_builtins::math::log2,
    math_log10 => tishlang_builtins::math::log10,
    math_exp => tishlang_builtins::math::exp,
    math_expm1 => tishlang_builtins::math::expm1,
    math_log1p => tishlang_builtins::math::log1p,
    math_cbrt => tishlang_builtins::math::cbrt,
    math_trunc => tishlang_builtins::math::trunc,
    math_sign => tishlang_builtins::math::sign,
    math_pow => tishlang_builtins::math::pow,
    math_max => tishlang_builtins::math::max,
    math_min => tishlang_builtins::math::min,
    math_hypot => tishlang_builtins::math::hypot,
    math_imul => tishlang_builtins::math::imul,
    math_clz32 => tishlang_builtins::math::clz32,
    math_fround => tishlang_builtins::math::fround,
    math_random => tishlang_builtins::math::random,
}

// Hyperbolic + inverse-hyperbolic: not in builtins::math; computed via FloatExt (libm,
// already in scope from the `pub use` above).
macro_rules! math_unary_libm {
    ($($name:ident => $m:ident),* $(,)?) => {
        $(
            #[inline]
            pub fn $name(args: &[Value]) -> Value {
                let n = match args.first() { Some(Value::Number(n)) => *n, _ => f64::NAN };
                Value::Number(n.$m())
            }
        )*
    };
}
math_unary_libm! {
    math_sinh => sinh,
    math_cosh => cosh,
    math_tanh => tanh,
    math_asinh => asinh,
    math_acosh => acosh,
    math_atanh => atanh,
}

pub mod gba;
