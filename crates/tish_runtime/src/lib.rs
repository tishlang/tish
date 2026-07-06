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
// `ObjectData`/`PropMap` are the concrete object representation. Re-exported so the
// native codegen can build an object literal's `PropMap` directly (pre-sized, single
// pass) instead of materializing an intermediate `AHashMap` and rebuilding from it.
pub use tishlang_core::{ObjectData, PropMap};
pub use tishlang_core::Value;
pub use tishlang_core::ArcStr;
/// Used by native codegen for `f()` / `obj()` dispatch (`Value::Function` or `__call` on objects).
/// Wraps `tishlang_core::value_call` with a pending-throw suppression check (#381): while a thrown
/// value is unwinding toward its checkpoint, calling anything would run side effects JS semantics
/// say must not happen after a throw (e.g. `console.log(f(x))` printing the NaN sentinel a tripped
/// recursion guard unwound with). One thread-local read on a path already dominated by boxing.
pub fn value_call(callee: &Value, args: &[Value]) -> Value {
    if has_pending_throw() {
        return Value::Null;
    }
    tishlang_core::value_call(callee, args)
}
/// JS ToInt32/ToUint32 for the emitted bitwise/shift code (modulo 2³², NaN/±Infinity → 0).
pub use tishlang_core::{
    to_int32, to_int32_value, to_number_value, to_uint32, to_uint32_value,
};
/// ECMAScript `Number::toString`, appended straight into a buffer — for in-place string building
/// (`s += n`) in emitted code, so a number append needs no throwaway `String`.
pub use tishlang_core::js_number_to_string_into;

/// Append `v`'s JS string-concatenation form directly to `buf` (no intermediate `String` for the
/// common scalar cases). Used by emitted `s += rhs` / `s = s + rhs` so a string-builder loop stays
/// allocation-light. Numbers go through `js_number_to_string_into` (JS-correct, ryu-backed), so a
/// concatenated float matches Node (`String(1e21)` → `"1e+21"`), which the old `n.to_string()`
/// Display path did not.
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
// Re-export the shared-mutable wrapper so the Rust code emitted by
// `tishlang_compile::codegen` can write `VmRef::new(...)` without needing
// a direct dependency on `tishlang_core` from the generated crate.
pub use tishlang_core::{VmReadGuard, VmRef, VmWriteGuard};

/// #218 — read a captured `VmRef` cell's value, releasing the lock guard BEFORE returning.
///
/// Generated native code reads captured cells in *expression position* (e.g. `current.dim +
/// current.fg` lowers to two reads of `current` inside one `+`). The old inline form
/// `(*cell.borrow()).clone()` puts the `borrow()` guard in a *temporary*, whose lifetime extends to
/// the end of the enclosing statement — so two reads of the SAME cell in one expression hold two
/// guards at once. Under the `send-values` build a `VmRef` is an `Arc<Mutex<T>>` and `borrow()` is a
/// non-reentrant `Mutex::lock()`, so that second lock self-deadlocks (the process sleeps at 0% CPU).
/// Cloning inside this fn drops the guard at the `return`, so repeated reads of one cell lock
/// strictly sequentially. Behaviour is identical to `(*cell.borrow()).clone()` otherwise.
#[inline]
pub fn vm_read<T: Clone>(cell: &VmRef<T>) -> T {
    (*cell.borrow()).clone()
}

/// `for…of` iterable normalization for the native backend: a JS iterator object (one with a
/// callable `next()` returning `{ value, done }`, e.g. a `Map`/`Set` `.values()` result) is
/// drained into a `Value::Array`; arrays, strings, and everything else pass through unchanged.
pub fn normalize_for_of(v: Value) -> Value {
    // A packed `NumberArray` (e.g. a module-const f64 array) must box to a plain `Value::Array` so the
    // single-variant `if let Value::Array` in the spread / for-of codegen binds — otherwise it spreads
    // to zero elements.
    if let Value::NumberArray(arr) = &v {
        let items: Vec<Value> = arr.borrow().iter().map(|n| Value::Number(*n)).collect();
        return Value::Array(VmRef::new(items));
    }
    match tishlang_core::drain_iterator(&v) {
        Some(items) => Value::Array(VmRef::new(items)),
        None => v,
    }
}

pub use tishlang_builtins::construct::array_construct;
pub use tishlang_builtins::construct::error_constructor_value as tish_error_constructor;
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
    collection_size, map_constructor_value as tish_map_constructor, map_get,
    map_has, map_set, map_values, set_constructor_value as tish_set_constructor,
};

// Re-export array methods from tishlang_builtins
pub use tishlang_builtins::array::{
    at as array_at, concat as array_concat_impl, every as array_every, filter as array_filter,
    find as array_find, find_index as array_find_index, find_last as array_find_last,
    find_last_index as array_find_last_index, flat as array_flat_impl, flat_map as array_flat_map,
    for_each as array_for_each, includes as array_includes_impl, index_of as array_index_of_impl,
    join as array_join_impl, map as array_map, pop as array_pop, push as array_push_impl,
    fill as array_fill, last_index_of as array_last_index_of, copy_within as array_copy_within,
    reduce as array_reduce, reduce_right as array_reduce_right, reverse as array_reverse,
    keys as array_keys, values as array_values, entries as array_entries, shift as array_shift,
    shuffle as array_shuffle, slice as array_slice_impl, snapshot_values as array_snapshot_values,
    as_f64_snapshot as array_as_f64_snapshot, some as array_some,
    sort_by_keys as array_sort_by_keys, sort_default as array_sort_default,
    sort_numeric_asc as array_sort_numeric_asc, sort_numeric_desc as array_sort_numeric_desc,
    sort_with_comparator as array_sort_with_comparator, splice as array_splice_impl,
    to_reversed as array_to_reversed, to_sorted as array_to_sorted,
    to_spliced as array_to_spliced, with as array_with,
    unshift as array_unshift_impl,
};

// Re-export string methods from tishlang_builtins
pub use tishlang_builtins::string::{
    at as string_at_impl, char_at as string_char_at_impl, char_code_at as string_char_code_at_impl,
    code_point_at as string_code_point_at,
    ends_with as string_ends_with_impl, escape_html as string_escape_html_impl,
    includes as string_includes_impl, index_of as string_index_of_impl,
    last_index_of as string_last_index_of_impl, pad_end as string_pad_end_impl,
    pad_start as string_pad_start_impl, repeat as string_repeat_impl,
    replace as string_replace_impl, replace_all as string_replace_all_impl,
    slice as string_slice_impl, split as string_split_impl, split_limit as string_split_limit_impl,
    starts_with as string_starts_with_impl, substr as string_substr_impl,
    substring as string_substring_impl,
    to_lower_case as string_to_lower_case, to_upper_case as string_to_upper_case,
    trim as string_trim, trim_end as string_trim_end, trim_start as string_trim_start,
};

// Wrapper functions to maintain API compatibility
pub fn array_push(arr: &Value, args: &[Value]) -> Value {
    array_push_impl(arr, args)
}
pub fn array_unshift(arr: &Value, args: &[Value]) -> Value {
    array_unshift_impl(arr, args)
}
pub fn array_index_of(arr: &Value, search: &Value, from: Option<&Value>) -> Value {
    array_index_of_impl(arr, search, from)
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
    string_split_limit(s, sep, &Value::Null)
}

/// `split(sep, limit)` honoring the optional `limit` argument (passed as a `Value` so the VM and
/// native codegen can forward the raw call argument). A non-numeric / null `limit` means "no limit".
/// Routes a RegExp separator to the regex splitter (matching string_replace's regex handling and
/// the interpreter), so `"a1b2c".split(RegExp("\\d",""))` works on the rust backend.
pub fn string_split_limit(s: &Value, sep: &Value, limit: &Value) -> Value {
    let max = match limit {
        Value::Number(n) if *n >= 0.0 => Some(*n as usize),
        _ => None,
    };
    #[cfg(feature = "regex")]
    if matches!(sep, Value::RegExp(_)) {
        return string_split_regex(s, sep, max);
    }
    string_split_limit_impl(s, sep, max)
}
pub fn string_starts_with(s: &Value, search: &Value, position: Option<&Value>) -> Value {
    string_starts_with_impl(s, search, position)
}
pub fn string_ends_with(s: &Value, search: &Value, end_position: Option<&Value>) -> Value {
    string_ends_with_impl(s, search, end_position)
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
pub fn string_at(s: &Value, idx: &Value) -> Value {
    string_at_impl(s, idx)
}
/// `.at(i)` dispatched on the runtime value — `at` exists on both String and Array, and the native
/// method match is by name (not receiver type), so route here at runtime. #247
pub fn value_at(recv: &Value, idx: &Value) -> Value {
    match recv {
        Value::String(_) => string_at_impl(recv, idx),
        _ => array_at(recv, idx),
    }
}
pub fn string_char_code_at(s: &Value, idx: &Value) -> Value {
    string_char_code_at_impl(s, idx)
}

// ── #317: typed-`String` (RustType::String) receiver fast paths ───────────────────────────────
//
// When native codegen knows the receiver of `s.charCodeAt(i)` / `s.charAt(i)` / `s.at(i)` / `s[i]`
// / `s.length` is a Rust `String` local (the `TISH_NATIVE_OPT` default), it can BORROW the string
// (`s.as_str()`) instead of deep-cloning it into a fresh `Value::String(ArcStr)` on every call. The
// boxed path's `s.clone().into()` is an O(n) copy of the whole string per call — O(n²) in a strided
// scan loop. These `&str` entry points eliminate that per-call copy.
//
// Indexing is by Unicode SCALAR (code point), byte-identical to `s.chars().nth(i)` /
// `s.chars().count()` — the same semantics as the boxed builtins (`tishlang_builtins::string`).
// `chars().nth(idx)` is O(idx) per call (acceptable: the win is removing the O(n) copy), versus the
// boxed cursor cache's O(1)-for-ASCII; correctness is identical either way.

#[inline]
fn idx_as_usize(idx: &Value) -> usize {
    match idx {
        Value::Number(n) => *n as usize,
        _ => 0,
    }
}

/// `&str` charCodeAt — code point at `idx` as f64; OOB → NaN. Mirrors `string::char_code_at`.
#[inline]
pub fn str_char_code_at(s: &str, idx: &Value) -> Value {
    match s.chars().nth(idx_as_usize(idx)) {
        Some(c) => Value::Number(c as u32 as f64),
        None => Value::Number(f64::NAN),
    }
}

/// `&str` charAt — 1-char string at `idx`; OOB → `""`. Mirrors `string::char_at`.
#[inline]
pub fn str_char_at(s: &str, idx: &Value) -> Value {
    match s.chars().nth(idx_as_usize(idx)) {
        Some(c) => Value::String(c.to_string().into()),
        None => Value::String("".into()),
    }
}

/// `&str` String.prototype.at — negative `idx` counts from the end; OOB → null. Mirrors `string::at`.
#[inline]
pub fn str_at(s: &str, idx: &Value) -> Value {
    let i = match idx {
        Value::Number(n) => *n as i64,
        _ => 0,
    };
    let resolved = if i < 0 {
        s.chars().count() as i64 + i
    } else {
        i
    };
    if resolved >= 0 {
        if let Some(c) = s.chars().nth(resolved as usize) {
            return Value::String(c.to_string().into());
        }
    }
    Value::Null
}

// ── O(1) char-slice forms: the loop-hoisted scan path ─────────────────────────────────────────
//
// `str_char_code_at` & friends index a `&str` via `chars().nth(i)` = O(i), so a strided scan
// (`for i in 0..s.length { s.charCodeAt(i) }`) is O(n²). When native codegen proves the String is
// loop-invariant it collects it ONCE into a `Vec<char>` and routes per-iteration accesses here, so
// each index is O(1) (scan → O(n)). Semantics are byte-identical to the `&str` forms: same
// `idx_as_usize`, `chars.len()` == `s.chars().count()`, `chars.get(i)` == `s.chars().nth(i)`.

/// O(1) char-slice `charCodeAt` — loop-hoisted [`str_char_code_at`]. OOB → NaN.
#[inline]
pub fn slice_char_code_at(chars: &[char], idx: &Value) -> Value {
    match chars.get(idx_as_usize(idx)) {
        Some(c) => Value::Number(*c as u32 as f64),
        None => Value::Number(f64::NAN),
    }
}

/// O(1) char-slice `charAt` — loop-hoisted [`str_char_at`]. OOB → `""`.
#[inline]
pub fn slice_char_at(chars: &[char], idx: &Value) -> Value {
    match chars.get(idx_as_usize(idx)) {
        Some(c) => Value::String(c.to_string().into()),
        None => Value::String("".into()),
    }
}

/// O(1) char-slice `at` (negative-index aware) — loop-hoisted [`str_at`]. OOB → null.
#[inline]
pub fn slice_at(chars: &[char], idx: &Value) -> Value {
    let i = match idx {
        Value::Number(n) => *n as i64,
        _ => 0,
    };
    let resolved = if i < 0 { chars.len() as i64 + i } else { i };
    if resolved >= 0 {
        if let Some(c) = chars.get(resolved as usize) {
            return Value::String(c.to_string().into());
        }
    }
    Value::Null
}

/// `&str` `s[i]` — char at a non-negative integer `idx`; non-int / negative / OOB → null.
/// Mirrors the `Value::String` arm of [`get_index`].
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

/// `&str` `.length` — Unicode scalar (code point) count as f64. Mirrors `string::char_count`.
#[inline]
pub fn str_char_count(s: &str) -> f64 {
    s.chars().count() as f64
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

/// `.toString([radix])` for the compiled backend (issue #59). A number receiver uses the
/// shared radix formatter so it stays byte-identical with the VM / interpreter; any other
/// receiver falls back to its normal JS string, so `[1,2].toString()` / `obj.toString()`
/// keep working.
pub fn number_to_string(n: &Value, radix: &Value) -> Value {
    match n {
        Value::Number(_) => tishlang_builtins::number::to_string(n, radix),
        other => Value::String(other.to_js_string().into()),
    }
}

/// Operators module for compound assignment operations
pub mod ops {
    use tishlang_core::Value;

    #[inline]
    // Arithmetic ops return a bare `Value` (not `Result`): they NEVER error — a non-number
    // operand coerces to NaN, matching the VM's `eval_binop`. The former `Result<Value, Box<dyn
    // Error>>` was vestigial and cost ~4× on the boxed path (memory-return + caller `.unwrap_or`;
    // #201 measurement). Callers use the value directly (no suffix — see `ops_result_suffix`).
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
            // Neither operand is a string ⇒ numeric coercion, matching the VM's `eval_binop`
            // (`as_number().unwrap_or(NaN)`): a null/bool/object operand (e.g. an out-of-bounds
            // array read) coerces to NaN, so `number + null` is NaN. Never an error.
            (a, b) => Value::Number(
                a.as_number().unwrap_or(f64::NAN) + b.as_number().unwrap_or(f64::NAN),
            ),
        }
    }

    #[inline]
    pub fn sub(left: &Value, right: &Value) -> Value {
        match (left, right) {
            (Value::Number(a), Value::Number(b)) => Value::Number(a - b),
            // VM-parity numeric coercion (null/non-number → NaN), see `add`.
            (a, b) => Value::Number(
                a.as_number().unwrap_or(f64::NAN) - b.as_number().unwrap_or(f64::NAN),
            ),
        }
    }

    #[inline]
    pub fn mul(left: &Value, right: &Value) -> Value {
        match (left, right) {
            (Value::Number(a), Value::Number(b)) => Value::Number(a * b),
            // VM-parity numeric coercion (null/non-number → NaN), see `add`.
            (a, b) => Value::Number(
                a.as_number().unwrap_or(f64::NAN) * b.as_number().unwrap_or(f64::NAN),
            ),
        }
    }

    #[inline]
    pub fn div(left: &Value, right: &Value) -> Value {
        match (left, right) {
            (Value::Number(a), Value::Number(b)) => Value::Number(a / b),
            // VM-parity numeric coercion (null/non-number → NaN), see `add`.
            (a, b) => Value::Number(
                a.as_number().unwrap_or(f64::NAN) / b.as_number().unwrap_or(f64::NAN),
            ),
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
    pub fn modulo(left: &Value, right: &Value) -> Value {
        match (left, right) {
            (Value::Number(a), Value::Number(b)) => Value::Number(a % b),
            // VM-parity numeric coercion (null/non-number → NaN), see `add`.
            (a, b) => Value::Number(
                a.as_number().unwrap_or(f64::NAN) % b.as_number().unwrap_or(f64::NAN),
            ),
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
    number_convert as builtins_number_convert,
    string_convert as builtins_string_convert,
    string_from_char_code as builtins_string_from_char_code,
};
use tishlang_core::{json_parse as core_json_parse, json_stringify as core_json_stringify};

/// Public JSON helpers used by codegen-emitted code (specifically the
/// `_tish_write_json` impls on user-declared `type` aliases). Re-exporting
/// from the runtime keeps the generated source decoupled from
/// `tishlang_core` — generated code only ever names `tishlang_runtime`.
pub mod json {
    pub use tishlang_core::json_parse;
    pub use tishlang_core::json_stringify_into as stringify_into;
    /// JS-correct number→JSON writer (NaN/Inf→`null`, integer fast path, else the
    /// ECMAScript `Number::toString`). Used by codegen-emitted per-struct serialisers
    /// (#315) for `number` fields, so their output matches `json_stringify_into` exactly.
    pub use tishlang_core::write_json_number;
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

// #303 — the pending-throw slot lives in `tishlang_core` so the shared array builtins
// (`tishlang_builtins::array`) can poll it without a `builtins -> runtime` dependency cycle, and so
// the VM shares the same slot. Re-export the accessors so the Rust emitted by `tishlang_compile`
// keeps calling `tishlang_runtime::{set,has,take}_pending_throw`. See `tishlang_core` for the docs.
pub use tishlang_core::{has_pending_throw, set_pending_throw, take_pending_throw};

// #381 — the recursion-guard plumbing for generated native code. The depth counter lives in
// `tishlang_core` beside the pending-throw slot; the entry point generated code uses is the
// wrapper below, which adds the stack-pressure check the counter alone can't provide.
pub use tishlang_core::{stack_overflow_error, CallDepthGuard};

/// #381 — enter a boxed user-fn call frame, or trip the recursion guard. Emitted at the top of
/// every generated user-fn closure. Trips on EITHER limit: the counted ceiling
/// (`TISH_MAX_CALL_DEPTH`, default 20000 — parity with the interp/VM guards) or real stack
/// pressure ([`stack_low`]) — boxed native frames are large enough that 20000 of them can exceed
/// the stack before the counter does (the VM sidesteps this with `stacker::maybe_grow`; generated
/// code has no stack growth, so pressure must be its own trigger). On trip: parks the catchable
/// `RangeError` and returns `None`; the closure returns its dummy `Value::Null` and the throw
/// surfaces at the caller's pending-throw checkpoint.
#[inline]
pub fn enter_call_guarded() -> Option<CallDepthGuard> {
    if stack_low() {
        set_pending_throw(stack_overflow_error());
        return None;
    }
    tishlang_core::enter_call_guarded()
}

#[cfg(not(target_family = "wasm"))]
thread_local! {
    // The current thread's bail floor for guarded self-recursive native fns: the stack address below
    // which recursion must stop (real stack bottom + headroom margin). 0 = not yet initialized.
    // Thread-local because every thread's stack occupies a different address range. #381
    static STACK_FLOOR: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Headroom left below the deepest allowed recursion level (#381): must cover the levels emitted
/// between two guard checks (the rotation window), the bail path, and building/raising the
/// `RangeError`. Mirrors the VM JIT tier's margin (`RECUR_STACK_MARGIN`).
#[cfg(not(target_family = "wasm"))]
const RECUR_STACK_MARGIN: usize = 256 * 1024;

/// #381 — is the current thread's stack nearly exhausted? Emitted by the native backend at the
/// entry of self-recursive typed fns (every K-th rotation copy), so unbounded numeric recursion
/// bails into a catchable `RangeError` instead of overflowing the stack (an uncatchable abort).
/// One thread-local read + pointer compare after first init; the margin is capped at half the
/// remaining stack so a first call on an already-deep stack can't false-trip (limit stays below SP).
#[cfg(not(target_family = "wasm"))]
#[inline]
pub fn stack_low() -> bool {
    let anchor = 0u8;
    let sp = &anchor as *const u8 as usize;
    STACK_FLOOR.with(|c| {
        let mut floor = c.get();
        if floor == 0 {
            // `None` (unknown bounds) → floor 1: SP can never be below it, guard never trips —
            // same do-no-harm fallback as the VM JIT tier.
            floor = match stacker::remaining_stack() {
                Some(rem) => {
                    let margin = RECUR_STACK_MARGIN.min(rem / 2);
                    sp.saturating_sub(rem).saturating_add(margin).max(1)
                }
                None => 1,
            };
            c.set(floor);
        }
        sp < floor
    })
}

/// Wasm: the sandbox traps on overflow (contained by design), and `stacker` has no wasm support —
/// the guard compiles to a constant `false` so the emitted check folds away.
#[cfg(target_family = "wasm")]
#[inline]
pub fn stack_low() -> bool {
    false
}

/// #381 — the bail path for a tripped typed-fn recursion guard: park the catchable `RangeError`
/// and return the NaN sentinel the f64 frame unwinds with. Typed native fns are pure numeric
/// (no side effects), so the NaN propagates harmlessly until the first `Value`/`Result` frame's
/// pending-throw checkpoint raises the error.
#[cold]
#[inline(never)]
pub fn recursion_tripped_f64() -> f64 {
    set_pending_throw(stack_overflow_error());
    f64::NAN
}

/// Function-boundary unwind: convert a completion that escaped a function body's `Result`-closure
/// back into the function's `Value`. A `return v` yields `v`; an uncaught `throw` is stored in the
/// pending-throw slot and the fn escapes with a dummy `Value` so the throw keeps propagating to the
/// caller's post-call check (#303); any other error panics.
pub fn fn_unwind(e: Box<dyn std::error::Error>) -> Value {
    match e.downcast::<TishError>() {
        Ok(te) => match *te {
            TishError::Return(v) => v,
            TishError::Throw(v) => {
                set_pending_throw(v);
                Value::Null
            }
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
pub fn encode_uri_component(args: &[Value]) -> Value {
    tishlang_builtins::globals::encode_uri_component(args)
}
pub fn decode_uri_component(args: &[Value]) -> Value {
    tishlang_builtins::globals::decode_uri_component(args)
}
pub fn structured_clone(args: &[Value]) -> Value {
    tishlang_builtins::globals::structured_clone(args)
}

// Math functions - use tishlang_builtins::math
pub use tishlang_builtins::math::{
    abs as tish_math_abs_impl, ceil as tish_math_ceil_impl, cos as tish_math_cos_impl,
    exp as tish_math_exp_impl, floor as tish_math_floor_impl, max as tish_math_max_impl,
    min as tish_math_min_impl, pow as tish_math_pow_impl, random as tish_math_random_impl,
    round as tish_math_round_impl, sign as tish_math_sign_impl, sin as tish_math_sin_impl,
    imul as tish_math_imul_impl,
    sqrt as tish_math_sqrt_impl, tan as tish_math_tan_impl, trunc as tish_math_trunc_impl,
    // hypot/atan2/asin/acos/atan were missing on the native Math but present on the vm (#247).
    hypot as tish_math_hypot_impl, atan2 as tish_math_atan2_impl, asin as tish_math_asin_impl,
    acos as tish_math_acos_impl, atan as tish_math_atan_impl,
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
pub fn math_hypot(args: &[Value]) -> Value {
    tish_math_hypot_impl(args)
}
pub fn math_atan2(args: &[Value]) -> Value {
    tish_math_atan2_impl(args)
}
pub fn math_asin(args: &[Value]) -> Value {
    tish_math_asin_impl(args)
}
pub fn math_acos(args: &[Value]) -> Value {
    tish_math_acos_impl(args)
}
pub fn math_atan(args: &[Value]) -> Value {
    tish_math_atan_impl(args)
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

// Hyperbolic / inverse-hyperbolic / cbrt / base-2/10 logs (issue #61). Compiled backends
// (native/cranelift/wasi) share this runtime, so wiring them here resolves all of them.
macro_rules! runtime_math_unary {
    ($name:ident, $method:ident) => {
        pub fn $name(args: &[Value]) -> Value {
            let n = extract_num(args.first()).unwrap_or(f64::NAN);
            Value::Number(n.$method())
        }
    };
}
runtime_math_unary!(math_sinh, sinh);
runtime_math_unary!(math_cosh, cosh);
runtime_math_unary!(math_tanh, tanh);
runtime_math_unary!(math_asinh, asinh);
runtime_math_unary!(math_acosh, acosh);
runtime_math_unary!(math_atanh, atanh);
runtime_math_unary!(math_cbrt, cbrt);
runtime_math_unary!(math_log2, log2);
runtime_math_unary!(math_log10, log10);
runtime_math_unary!(math_expm1, exp_m1);
runtime_math_unary!(math_log1p, ln_1p);
pub fn math_clz32(args: &[Value]) -> Value {
    tishlang_builtins::math::clz32(args)
}
pub fn math_fround(args: &[Value]) -> Value {
    tishlang_builtins::math::fround(args)
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
pub fn array_of(args: &[Value]) -> Value {
    tishlang_builtins::globals::array_of(args)
}
pub fn array_from(args: &[Value]) -> Value {
    tishlang_builtins::globals::array_from(args)
}
pub fn object_is(args: &[Value]) -> Value {
    tishlang_builtins::globals::object_is(args)
}

pub fn string_from_char_code(args: &[Value]) -> Value {
    builtins_string_from_char_code(args)
}

/// `String(value)` as a function (JS `ToString`). Wired into the codegen `String`
/// global as `__call` so compiled `String(x)` matches the VM/interp.
pub fn string_convert(args: &[Value]) -> Value {
    builtins_string_convert(args)
}

pub fn number_convert(args: &[Value]) -> Value {
    builtins_number_convert(args)
}
pub fn number_is_integer(args: &[Value]) -> Value {
    tishlang_builtins::globals::number_is_integer(args)
}
pub fn number_is_safe_integer(args: &[Value]) -> Value {
    tishlang_builtins::globals::number_is_safe_integer(args)
}
pub fn number_is_nan(args: &[Value]) -> Value {
    tishlang_builtins::globals::number_is_nan(args)
}
pub fn number_is_finite(args: &[Value]) -> Value {
    tishlang_builtins::globals::number_is_finite(args)
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

/// `process.execFile(program, [args])` — run a program directly, WITHOUT a shell (no `sh -c`). Each
/// argument is passed to the program verbatim, so shell metacharacters in untrusted argument data are
/// never interpreted — the safe counterpart to `exec` when arguments derive from input (#384). Returns
/// the exit code, like `exec`.
#[cfg(feature = "process")]
pub fn process_exec_file(args: &[Value]) -> Value {
    use std::process::Command;
    let program = args
        .first()
        .map(|v| v.to_display_string())
        .unwrap_or_default();
    if program.is_empty() {
        return Value::Number(0.0);
    }
    let argv: Vec<String> = match args.get(1) {
        Some(Value::Array(a)) => a.borrow().iter().map(|v| v.to_display_string()).collect(),
        _ => Vec::new(),
    };
    match Command::new(&program).args(&argv).status() {
        Ok(status) => Value::Number(status.code().unwrap_or(1) as f64),
        Err(_) => Value::Number(1.0),
    }
}

#[cfg(all(test, feature = "process", unix))]
mod execfile_tests_384 {
    use super::process_exec_file;
    use tishlang_core::{Value, VmRef};

    #[test]
    fn execfile_runs_without_shell_and_returns_exit_code() {
        // Runs the program directly (no `sh -c`); `true`/`false`/`echo` exist on unix.
        assert!(matches!(process_exec_file(&[Value::String("true".into())]), Value::Number(n) if n == 0.0));
        assert!(matches!(process_exec_file(&[Value::String("false".into())]), Value::Number(n) if n == 1.0));
        // args are passed verbatim as a Value::Array.
        let args = Value::Array(VmRef::new(vec![Value::String("ok".into())]));
        assert!(matches!(process_exec_file(&[Value::String("echo".into()), args]), Value::Number(n) if n == 0.0));
        // empty program is a no-op returning 0, matching `exec`.
        assert!(matches!(process_exec_file(&[]), Value::Number(n) if n == 0.0));
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

/// Read a file as raw bytes — an array of numbers 0–255 — for binary data that `read_file`
/// (UTF-8 only) can't handle (images, fonts, archives). Lets pure-Tish decoders work on local
/// files. See issue #120.
#[cfg(feature = "fs")]
pub fn read_file_bytes(args: &[Value]) -> Value {
    let path = args
        .first()
        .map(|v| v.to_display_string())
        .unwrap_or_default();
    match std::fs::read(&path) {
        Ok(bytes) => Value::Array(VmRef::new(
            bytes.into_iter().map(|b| Value::Number(b as f64)).collect(),
        )),
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

/// Per-site polymorphic inline cache for `obj.<literal key>` reads in generated native code (#179).
///
/// The native backend otherwise pays a full dynamic lookup (RefCell/Mutex borrow + linear key scan)
/// for every member read, even at sites that only ever see a few object shapes. `PropMap` already
/// tracks a hidden-class [`ShapeId`] per object and exposes slot-indexed access, so once a
/// `(shape, slot)` pair is observed at a site, a later object of the same shape resolves the property
/// with one integer compare + a direct slot load instead of a key scan.
///
/// Each of the 8 entries packs `(shape: u32) << 32 | (slot: u32)` into a single `AtomicU64`, so the
/// pair is read and written atomically — no torn `(shape, slot)` under concurrent access (e.g. an
/// HTTP handler shared across worker threads). A 9th distinct shape evicts an entry round-robin. An
/// empty entry is `0`; a real object never has `EMPTY_SHAPE` (0), so `0` can't be a false hit.
pub struct PropIC {
    entries: [std::sync::atomic::AtomicU64; 8],
    next: std::sync::atomic::AtomicU32,
}

impl PropIC {
    #[allow(clippy::declare_interior_mutable_const)] // each `static IC: PropIC = PropIC::new()` site needs its own cell
    pub const fn new() -> Self {
        use std::sync::atomic::{AtomicU32, AtomicU64};
        PropIC {
            entries: [
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
            ],
            next: AtomicU32::new(0),
        }
    }
}

impl Default for PropIC {
    fn default() -> Self {
        Self::new()
    }
}

/// Cached `obj.key` read — see [`PropIC`]. Behaviour is identical to [`get_prop`]: only objects with
/// a stable hidden class (non-empty, non-dictionary) take the cached fast path; empty/dictionary
/// objects, non-objects, and the special `size` key all fall through to `get_prop` (the borrow is
/// released first, so the fallback's re-borrow can't self-deadlock the send-values `Mutex`). The
/// caller (codegen) never emits this for the `size` key, but the fallback covers it regardless.
#[inline]
pub fn get_prop_ic(obj: &Value, key: &str, ic: &PropIC) -> Value {
    use std::sync::atomic::Ordering;
    if key != "size" {
        if let Value::Object(map) = obj {
            // Scope the borrow so it is released before the `get_prop` fallback below.
            {
                let b = map.borrow();
                let shape = b.strings.shape();
                if shape != tishlang_core::EMPTY_SHAPE && shape != tishlang_core::DICT_SHAPE {
                    let shape_hi = (shape as u64) << 32;
                    // Fast path: a cached entry whose shape matches gives the slot directly.
                    for e in ic.entries.iter() {
                        let packed = e.load(Ordering::Relaxed);
                        if packed >> 32 == shape as u64 {
                            if let Some(v) = b.strings.value_at_index((packed & 0xFFFF_FFFF) as usize)
                            {
                                return v.clone();
                            }
                        }
                    }
                    // Miss: resolve once, fill an entry round-robin with the packed (shape, slot).
                    return match b.strings.get_with_index(key) {
                        Some((v, i)) => {
                            let k = (ic.next.fetch_add(1, Ordering::Relaxed) % 8) as usize;
                            ic.entries[k].store(shape_hi | i as u64, Ordering::Relaxed);
                            v.clone()
                        }
                        None => Value::Null,
                    };
                }
                // empty / dictionary object → fall through (borrow dropped here)
            }
        }
    }
    get_prop(obj, key)
}

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
                Value::Number(tishlang_builtins::string::char_count(s) as f64)
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
        // Reading a property of the nullish value is a JS TypeError. PARK a catchable throw (#425)
        // and return the null sentinel; it surfaces at the next pending-throw checkpoint. This is the
        // ONLY receiver that throws — a number/bool/function with no such property reads back `null`
        // (JS `undefined`), matching the VM/interpreter/node. Free for valid reads: `Object` is matched
        // first, so this cold arm never runs on the hot path.
        Value::Null => {
            tishlang_core::set_pending_throw(tishlang_core::cannot_read_property_error(key));
            Value::Null
        }
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
        // `str[i]` returns the character at index `i` (issue #17) — matches the VM /
        // interpreter; out-of-bounds / negative / non-integer indices yield null.
        Value::String(s) => match index {
            Value::Number(n) if *n >= 0.0 && n.fract() == 0.0 => {
                tishlang_builtins::string::nth_char(s, *n as usize)
                    .map(|c| Value::String(c.to_string().into()))
                    .unwrap_or(Value::Null)
            }
            _ => Value::Null,
        },
        Value::Object(_) => tishlang_core::object_get(obj, index).unwrap_or(Value::Null),
        // `promise["then"|"catch"]` — string-keyed access mirrors `get_prop` (bracket form
        // is required for `catch`, a reserved word). Keeps the rust backend on par with the VM.
        #[cfg(any(feature = "http", feature = "promise"))]
        Value::Promise(_) => match index {
            Value::String(k) => get_prop(obj, k.as_str()),
            _ => Value::Null,
        },
        // Indexing the nullish value throws a catchable TypeError (#425), like `get_prop` above —
        // parked and surfaced at the next checkpoint. Every other receiver reads back `null`.
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

/// `delete obj[key]` / `delete obj.prop` (issue #40). Objects drop the string key; arrays
/// clear a numeric index to a `null` hole (length preserved). Always evaluates to `true`.
#[inline]
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
        // `arr.length = k` truncates / grows the array (holes read back as Null), matching
        // JS and the VM/interpreter (issue #62).
        Value::Array(arr) if key == "length" => {
            let n = extract_num(Some(&val)).unwrap_or(f64::NAN);
            if n.is_nan() || n < 0.0 || n.fract() != 0.0 || n > 4_294_967_295.0 {
                panic!("Invalid array length");
            }
            arr.borrow_mut().resize(n as usize, Value::Null);
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

#[cfg(feature = "tty")]
pub mod tty;
#[cfg(feature = "tty")]
pub use tty::{
    tty_enter_alt_screen, tty_is_tty, tty_leave_alt_screen, tty_read, tty_read_line, tty_set_raw_mode, tty_size,
};

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
            // JS semantics: split fully, then truncate to `limit` (don't keep the unsplit remainder
            // in the last slot). `truncate(usize::MAX)` is a no-op, so the no-limit path is unchanged.
            let mut result = Vec::new();
            let mut last_end = 0;

            for mat in re.regex.find_iter(input) {
                match mat {
                    Ok(m) => {
                        result.push(Value::String(input[last_end..m.start()].into()));
                        last_end = m.end();
                    }
                    Err(_) => break,
                }
            }
            result.push(Value::String(input[last_end..].into()));
            result.truncate(max);

            Value::Array(VmRef::new(result))
        }
        Value::String(sep) => {
            let mut parts: Vec<Value> = input
                .split(sep.as_str())
                .map(|s| Value::String(s.into()))
                .collect();
            parts.truncate(max);
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

#[cfg(test)]
mod null_read_parking_tests_425 {
    // Reading a property/index of the nullish value PARKS a catchable TypeError (#425) instead of
    // silently reading back `null` — the native/runtime read paths. The throw surfaces at the caller's
    // next pending-throw checkpoint. Every other receiver (number/bool/valid object/array) reads back
    // a value with NO parked throw.
    use super::{get_index, get_prop};
    use tishlang_core::{has_pending_throw, take_pending_throw, Value, VmRef};

    fn parked_name() -> Option<String> {
        take_pending_throw().and_then(|v| {
            if let Value::Object(o) = v {
                if let Some(Value::String(s)) = o.borrow().strings.get("name") {
                    return Some(s.to_string());
                }
            }
            None
        })
    }

    #[test]
    fn get_prop_on_null_parks_type_error() {
        let _ = take_pending_throw();
        let r = get_prop(&Value::Null, "length");
        assert!(matches!(r, Value::Null));
        assert!(has_pending_throw());
        assert_eq!(parked_name().as_deref(), Some("TypeError"));
    }

    #[test]
    fn get_index_on_null_parks_type_error() {
        let _ = take_pending_throw();
        let r = get_index(&Value::Null, &Value::Number(0.0));
        assert!(matches!(r, Value::Null));
        assert!(has_pending_throw());
        assert_eq!(parked_name().as_deref(), Some("TypeError"));
    }

    #[test]
    fn valid_reads_do_not_park() {
        let _ = take_pending_throw();
        // object property, array index, and array length must NOT park a throw.
        let obj = Value::object({
            let mut m = tishlang_core::ObjectMap::default();
            m.insert(std::sync::Arc::from("a"), Value::Number(42.0));
            m
        });
        assert!(matches!(get_prop(&obj, "a"), Value::Number(n) if n == 42.0));
        assert!(!has_pending_throw(), "valid property read must not park");
        let arr = Value::Array(VmRef::new(vec![Value::Number(7.0)]));
        assert!(matches!(get_index(&arr, &Value::Number(0.0)), Value::Number(n) if n == 7.0));
        assert!(!has_pending_throw(), "valid index read must not park");
        // a MISSING object property reads back null WITHOUT parking (JS `undefined`, not a throw).
        assert!(matches!(get_prop(&obj, "missing"), Value::Null));
        assert!(!has_pending_throw(), "missing property must not park");
    }
}
