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
    to_uint32_value, ArcStr, NumArrayBacking, ObjectData, ObjectMap, PropMap, TishStruct, Value,
    VmReadGuard, VmRef, VmWriteGuard,
};
/// `Arc` on GBA is `Rc` (single-core). Emitted code writes `Arc::from(..)`.
pub use tishlang_core::Arc;
/// Fixed-point scalar the typed-numeric path lowers `fixed` to (agb `Num<i32,8>`).
pub type Fixed = agb::fixnum::Num<i32, 8>;

// ── Pending-throw + recursion-guard plumbing (shared slot in tishlang_core) ──
pub use tishlang_core::{
    has_pending_throw, set_pending_throw, stack_overflow_error, take_pending_throw, CallDepthGuard,
};

unsafe extern "C" {
    /// End of the `.iwram` output section — provided by agb's `gba.ld`. IWRAM runs
    /// 0x0300_0000..0x0300_8000; agb's code/data occupies the bottom and the user
    /// stack grows DOWN from 0x0300_7F00 toward this address. Anything below it is
    /// live agb data (the audio mixer's buffers among them), so this is the real
    /// bottom of the stack.
    static __iwram_end: u8;
}

/// Headroom kept above [`__iwram_end`] (#655): must cover the frames emitted between
/// two guard checks plus the bail path that parks the `RangeError`. Boxed `Value`
/// frames on GBA run a few hundred bytes each, and the total stack is under 32 KB,
/// so this is deliberately small relative to the host's 256 KB margin.
const GBA_STACK_MARGIN: usize = 2 * 1024;

/// #655 — is the GBA stack nearly exhausted? The host probe (`stacker::remaining_stack`)
/// needs `std` and reports no bounds here, so its `None → floor 1` fallback made the
/// guard dead code on the one target where overflow is unrecoverable: there is no MMU
/// and no guard page, so the stack silently grows down into agb's IWRAM data and the
/// corruption surfaces frames later as an illegal opcode somewhere unrelated.
///
/// The bounds ARE known statically here — the linker hands us the IWRAM extent — so
/// this is a link-time constant plus a pointer compare, about the cost of the
/// thread-local read it replaces on the host.
#[inline]
pub fn stack_low() -> bool {
    let anchor = 0u8;
    let sp = &anchor as *const u8 as usize;
    let floor = (&raw const __iwram_end) as usize + GBA_STACK_MARGIN;
    sp < floor
}

/// Enter a boxed user-fn call frame, or trip the recursion guard. Trips on EITHER the
/// counted ceiling (`tishlang_core`) or real stack pressure ([`stack_low`]) — on a
/// 32 KB IWRAM stack the counted default is far out of reach, so pressure is the
/// trigger that actually fires. On trip: parks the catchable `RangeError` and returns
/// `None`, exactly as on the host.
#[inline]
pub fn enter_call_guarded() -> Option<CallDepthGuard> {
    if stack_low() {
        set_pending_throw(stack_overflow_error());
        return None;
    }
    tishlang_core::enter_call_guarded()
}

/// #655 — bail path for a tripped typed-fn recursion guard (GBA mirror of the host's
/// `recursion_tripped_f64`): park the catchable `RangeError` and unwind the numeric
/// frame with NaN until the first `Value` frame's pending-throw checkpoint raises it.
#[cold]
#[inline(never)]
pub fn recursion_tripped_f64() -> f64 {
    set_pending_throw(stack_overflow_error());
    f64::NAN
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
        // A by-reference boxed typed struct: `"field" in s` materialises to the struct's object and
        // asks it, so a boxed struct answers `in` like the object it stands for (was always `false`).
        Value::Struct(s) => {
            Value::Bool(tishlang_core::object_has(&s.borrow().tish_to_object(), key))
        }
        _ => Value::Bool(false),
    }
}

/// Spread source for `{ ...v }` (GBA emit only — codegen calls this from the Gba object-spread
/// lowering). Returns the source's own string-keyed props as an owned `PropMap`: a boxed typed struct
/// is materialised to its ordered object first, so `{ ...s }` carries the struct's fields instead of
/// dropping them; a non-object yields `None` (spread of `null`/a number contributes nothing, per JS).
/// Desktop emit keeps its inline `if let Value::Object` path, so this GBA-only helper adds no
/// non-GBA cost.
pub fn object_spread_props(v: &Value) -> Option<PropMap> {
    match v {
        Value::Object(o) => Some(o.borrow().strings.clone()),
        Value::Struct(s) => match s.borrow().tish_to_object() {
            Value::Object(o) => Some(o.borrow().strings.clone()),
            _ => None,
        },
        _ => None,
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
        // A boxed typed struct: read the field natively (a small match, no hashmap). `size`/method
        // sugar isn't relevant here — structs are plain field bags.
        Value::Struct(s) => s.borrow().tish_get(key),
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
        Value::Struct(s) => {
            let key = match index {
                Value::String(k) => k.to_string(),
                other => other.to_js_string(),
            };
            s.borrow().tish_get(&key)
        }
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

/// `&str` `s[i]` — char at a non-negative integer `idx`; non-int / negative / OOB → null.
///
/// Mirrors `tish_runtime::str_index`, and exists here because codegen emits a call to it whenever
/// the indexed expression is statically a `&str` rather than a `Value` — which is what a module
/// `const` string lowers to. Without it, `const D = "0123456789"` followed by `D[i]` is a hard
/// E0425 on the GBA target only, while the identical code compiles and runs everywhere else.
#[inline]
pub fn str_index(s: &str, idx: &Value) -> Value {
    match idx {
        Value::Number(n) if *n >= 0.0 && n.fract() == 0.0 => match s.chars().nth(*n as usize) {
            Some(c) => Value::String(c.to_string().into()),
            None => Value::Null,
        },
        _ => Value::Null,
    }
}

/// `&str` charCodeAt — the code unit at `idx`, NaN when out of range. Mirrors
/// `tish_runtime::str_char_code_at`, and missing here for the same reason `str_index` was: codegen
/// emits it whenever the receiver is statically a `&str`, so `"abc".charCodeAt(i)` was a hard E0425
/// on the GBA target only while compiling everywhere else. Found by a ROM that hashes its own trace.
#[inline]
pub fn str_char_code_at(s: &str, idx: &Value) -> Value {
    let i = match idx {
        Value::Number(n) if *n >= 0.0 && n.fract() == 0.0 => *n as usize,
        _ => return Value::Number(f64::NAN),
    };
    match s.chars().nth(i) {
        Some(c) => Value::Number(c as u32 as f64),
        None => Value::Number(f64::NAN),
    }
}

/// `&str` charAt — the 1-character string at `idx`, `""` out of range. Same reason as above.
#[inline]
pub fn str_char_at(s: &str, idx: &Value) -> Value {
    let i = match idx {
        Value::Number(n) if *n >= 0.0 && n.fract() == 0.0 => *n as usize,
        _ => return Value::String("".into()),
    };
    match s.chars().nth(i) {
        Some(c) => Value::String(c.to_string().into()),
        None => Value::String("".into()),
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
        Value::Struct(s) => {
            s.borrow_mut().tish_set(key, val.clone());
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

/// `json::{escape_into, write_json_number}` — the helpers codegen-emitted per-struct JSON serialisers
/// (`__tish_json_TishStruct_*`, #315) call. The std runtime exposes these under `tishlang_runtime::json`;
/// the GBA facade must mirror them so a game that uses a typed `interface`/`type` compiles. no_std.
pub mod json {
    pub use tishlang_core::write_json_number;

    /// Append the JSON-escaped contents of `s` (no surrounding quotes) to `buf`. Same escape rules as
    /// the std runtime's `json::escape_into`; kept local (uses `core::fmt::Write` for `\uXXXX`).
    pub fn escape_into(buf: &mut alloc::string::String, s: &str) {
        use core::fmt::Write;
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

// ── String methods (codegen lowers `.slice()`/`.indexOf()`/`.toLowerCase()`/… to these prelude
// names). Mirrors the std `tish_runtime` wrappers; the portable subset (no regex / unicode-normalize).
pub use tishlang_builtins::string::{
    at as string_at_impl, char_at as string_char_at_impl,
    char_code_at as string_char_code_at_impl, ends_with as string_ends_with_impl,
    includes as string_includes_impl, index_of as string_index_of_impl,
    slice as string_slice_impl, starts_with as string_starts_with_impl,
    substr as string_substr_impl, substring as string_substring_impl,
    to_lower_case as string_to_lower_case, to_upper_case as string_to_upper_case,
    trim as string_trim, trim_end as string_trim_end, trim_start as string_trim_start,
};
#[inline]
pub fn string_index_of(s: &Value, search: &Value, from: &Value) -> Value {
    string_index_of_impl(s, search, Some(from))
}
#[inline]
pub fn string_includes(s: &Value, search: &Value, from: &Value) -> Value {
    string_includes_impl(s, search, Some(from))
}
#[inline]
pub fn string_slice(s: &Value, start: &Value, end: &Value) -> Value {
    string_slice_impl(s, start, end)
}
#[inline]
pub fn string_substring(s: &Value, start: &Value, end: &Value) -> Value {
    string_substring_impl(s, start, end)
}
#[inline]
pub fn string_substr(s: &Value, start: &Value, length: &Value) -> Value {
    string_substr_impl(s, start, length)
}
#[inline]
pub fn string_starts_with(s: &Value, search: &Value, position: Option<&Value>) -> Value {
    string_starts_with_impl(s, search, position)
}
#[inline]
pub fn string_ends_with(s: &Value, search: &Value, end_position: Option<&Value>) -> Value {
    string_ends_with_impl(s, search, end_position)
}
#[inline]
pub fn string_char_at(s: &Value, idx: &Value) -> Value {
    string_char_at_impl(s, idx)
}
#[inline]
pub fn string_at(s: &Value, idx: &Value) -> Value {
    string_at_impl(s, idx)
}
#[inline]
pub fn string_char_code_at(s: &Value, idx: &Value) -> Value {
    string_char_code_at_impl(s, idx)
}

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
// Array methods — codegen lowers `.push()`/`.slice()`/`.map()`/… to these `array_*` names. Mirrors
// the std `tish_runtime` block (minus RNG-dependent `shuffle`). Portable: all operate on Vec<Value>.
pub use tishlang_builtins::array::{
    at as array_at, concat as array_concat_impl, every as array_every, filter as array_filter,
    find as array_find, find_index as array_find_index, find_last as array_find_last,
    find_last_index as array_find_last_index, flat as array_flat_impl, flat_map as array_flat_map,
    for_each as array_for_each, includes as array_includes_impl, index_of as array_index_of_impl,
    join as array_join_impl, map as array_map, pop as array_pop, push as array_push_impl,
    fill as array_fill, last_index_of as array_last_index_of, copy_within as array_copy_within,
    reduce as array_reduce, reduce_right as array_reduce_right, reverse as array_reverse,
    keys as array_keys, values as array_values, entries as array_entries, shift as array_shift,
    slice as array_slice_impl, snapshot_values as array_snapshot_values,
    as_f64_snapshot as array_as_f64_snapshot, some as array_some,
    sort_by_keys as array_sort_by_keys, sort_default as array_sort_default,
    sort_numeric_asc as array_sort_numeric_asc, sort_numeric_desc as array_sort_numeric_desc,
    sort_with_comparator as array_sort_with_comparator, splice as array_splice_impl,
    to_reversed as array_to_reversed, to_sorted as array_to_sorted,
    to_spliced as array_to_spliced, with as array_with, unshift as array_unshift_impl,
};
#[inline]
pub fn array_push(arr: &Value, args: &[Value]) -> Value { array_push_impl(arr, args) }
#[inline]
pub fn array_unshift(arr: &Value, args: &[Value]) -> Value { array_unshift_impl(arr, args) }
#[inline]
pub fn array_index_of(arr: &Value, search: &Value, from: Option<&Value>) -> Value {
    array_index_of_impl(arr, search, from)
}
#[inline]
pub fn array_includes(arr: &Value, search: &Value, from: &Value) -> Value {
    array_includes_impl(arr, search, Some(from))
}
#[inline]
pub fn array_join(arr: &Value, sep: &Value) -> Value { array_join_impl(arr, sep) }
#[inline]
pub fn array_splice(arr: &Value, start: &Value, delete_count: Option<&Value>, items: &[Value]) -> Value {
    array_splice_impl(arr, start, delete_count, items)
}
#[inline]
pub fn array_slice(arr: &Value, start: &Value, end: &Value) -> Value {
    array_slice_impl(arr, start, end)
}
#[inline]
pub fn array_concat(arr: &Value, args: &[Value]) -> Value { array_concat_impl(arr, args) }
#[inline]
pub fn array_flat(arr: &Value, depth: &Value) -> Value { array_flat_impl(arr, depth) }
#[inline]
pub fn array_sort(arr: &Value, comparator: Option<&Value>) -> Value {
    match comparator {
        Some(cmp) => array_sort_with_comparator(arr, cmp),
        None => array_sort_default(arr),
    }
}
/// `.at(i)` dispatched on the runtime value — exists on both String and Array (#247).
#[inline]
pub fn value_at(recv: &Value, idx: &Value) -> Value {
    match recv {
        Value::String(_) => string_at_impl(recv, idx),
        _ => array_at(recv, idx),
    }
}
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
