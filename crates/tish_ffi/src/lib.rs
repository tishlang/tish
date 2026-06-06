//! # tish FFI — the stable C ABI for native extensions (Workstream B / B1)
//!
//! `tish_core::Value` is a non-`#[repr(C)]` Rust enum, so it cannot cross an `extern "C"`
//! boundary by value, and a native extension compiled separately can't name its type
//! ("conflicting Value types"). This crate defines the **opaque-handle + accessor** ABI that
//! decouples extensions from `tish_core`:
//!
//! - A [`TishValueRef`] is an opaque handle (a boxed `Value` behind a `*mut c_void`). An
//!   extension only ever sees the pointer, never the layout.
//! - The host exposes the `extern "C"` accessors below (`tish_value_*`); an extension *imports*
//!   them. A native extension is then a **cdylib** whose exports match [`TishNativeFn`], and the
//!   exact same contract is satisfied on wasm by **host imports** — one ABI, two bindings
//!   (B4). This is what lets cranelift/llvm/wasi load native extensions without sharing a Rust
//!   compilation; only `cargo:` (Rust-crate compile-time linking) stays rust-AOT-only.
//!
//! Ownership rule (C-style): every handle a caller *receives* from a `_new_*` / `_get` / `_clone`
//! accessor is owned by the caller and must be released with [`tish_value_drop`]; `_push`/`_set`
//! **clone** their argument (the caller keeps owning it). Strings returned by
//! [`tish_value_as_string`] are owned by the caller and freed with [`tish_string_free`].
//!
//! **Status:** B1 (this crate) is the ABI + host accessors, unit-tested below. The loader
//! (`libloading` cdylib / wasm host imports) and per-backend `ffi:` wiring are B2–B4.

use std::ffi::{c_char, c_void, CStr, CString};

use tishlang_core::{ObjectData, Value, VmRef};

/// Opaque handle to a tish value. Internally `*mut Value` (a leaked `Box`); never inspect or
/// free it except through the accessors. `null` is a valid "no value" sentinel for fallible
/// accessors (e.g. `tish_value_object_get` on a missing key returns a fresh null handle, not
/// a null pointer — but defensive code treats a null pointer as `Value::Null`).
pub type TishValueRef = *mut c_void;

/// Value kind tags returned by [`tish_value_tag`]. Stable across versions.
pub const TISH_TAG_NULL: i32 = 0;
pub const TISH_TAG_NUMBER: i32 = 1;
pub const TISH_TAG_STRING: i32 = 2;
pub const TISH_TAG_BOOL: i32 = 3;
pub const TISH_TAG_ARRAY: i32 = 4;
pub const TISH_TAG_OBJECT: i32 = 5;
/// Anything the C ABI doesn't model (Function/Promise/RegExp/Symbol/…). Opaque to extensions.
pub const TISH_TAG_OTHER: i32 = 6;

/// An `extern "C"` native function: receives a borrowed array of argument handles (owned by the
/// host for the call) and returns a freshly-owned result handle (the host drops it).
pub type TishNativeFn =
    extern "C" fn(args: *const TishValueRef, argc: usize) -> TishValueRef;

/// One named export in a module's table.
#[repr(C)]
pub struct TishExport {
    /// NUL-terminated function name.
    pub name: *const c_char,
    pub func: TishNativeFn,
}

/// What a module's `#[no_mangle] extern "C" fn tish_module_register() -> *const TishExportTable`
/// returns: a static name→fn table the host registers as a native module.
#[repr(C)]
pub struct TishExportTable {
    pub exports: *const TishExport,
    pub count: usize,
}

// ── handle <-> Value plumbing ────────────────────────────────────────────────
#[inline]
fn box_value(v: Value) -> TishValueRef {
    Box::into_raw(Box::new(v)) as TishValueRef
}

/// Borrow the `Value` behind a handle; `None` for a null pointer (treated as `Value::Null`).
///
/// # Safety
/// `r` must be null or a handle returned by an accessor and not yet dropped.
#[inline]
unsafe fn as_value<'a>(r: TishValueRef) -> Option<&'a Value> {
    (r as *const Value).as_ref()
}

// ── constructors ─────────────────────────────────────────────────────────────
#[no_mangle]
pub extern "C" fn tish_value_new_number(n: f64) -> TishValueRef {
    box_value(Value::Number(n))
}

#[no_mangle]
pub extern "C" fn tish_value_new_bool(b: bool) -> TishValueRef {
    box_value(Value::Bool(b))
}

#[no_mangle]
pub extern "C" fn tish_value_new_null() -> TishValueRef {
    box_value(Value::Null)
}

/// Build a string value from a NUL-terminated UTF-8 C string. Returns a null value handle if
/// `s` is null or not valid UTF-8.
///
/// # Safety
/// `s` must be null or point to a valid NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn tish_value_new_string(s: *const c_char) -> TishValueRef {
    if s.is_null() {
        return box_value(Value::Null);
    }
    match CStr::from_ptr(s).to_str() {
        Ok(st) => box_value(Value::String(st.into())),
        Err(_) => box_value(Value::Null),
    }
}

// ── tag + scalar readers ─────────────────────────────────────────────────────
/// # Safety: `r` must be null or a live handle.
#[no_mangle]
pub unsafe extern "C" fn tish_value_tag(r: TishValueRef) -> i32 {
    match as_value(r) {
        None | Some(Value::Null) => TISH_TAG_NULL,
        Some(Value::Number(_)) => TISH_TAG_NUMBER,
        Some(Value::String(_)) => TISH_TAG_STRING,
        Some(Value::Bool(_)) => TISH_TAG_BOOL,
        Some(Value::Array(_)) => TISH_TAG_ARRAY,
        Some(Value::Object(_)) => TISH_TAG_OBJECT,
        Some(_) => TISH_TAG_OTHER,
    }
}

/// Number value, or `NaN` if the handle isn't a number.
/// # Safety: `r` must be null or a live handle.
#[no_mangle]
pub unsafe extern "C" fn tish_value_as_number(r: TishValueRef) -> f64 {
    match as_value(r) {
        Some(Value::Number(n)) => *n,
        _ => f64::NAN,
    }
}

/// Bool value, or `false` if the handle isn't a bool.
/// # Safety: `r` must be null or a live handle.
#[no_mangle]
pub unsafe extern "C" fn tish_value_as_bool(r: TishValueRef) -> bool {
    matches!(as_value(r), Some(Value::Bool(true)))
}

/// Newly-allocated NUL-terminated copy of a string value (caller frees with
/// [`tish_string_free`]); null pointer if the handle isn't a string (or contains an interior NUL).
/// # Safety: `r` must be null or a live handle.
#[no_mangle]
pub unsafe extern "C" fn tish_value_as_string(r: TishValueRef) -> *mut c_char {
    match as_value(r) {
        Some(Value::String(s)) => match CString::new(s.as_bytes()) {
            Ok(c) => c.into_raw(),
            Err(_) => std::ptr::null_mut(),
        },
        _ => std::ptr::null_mut(),
    }
}

/// Free a string returned by [`tish_value_as_string`].
/// # Safety: `s` must be null or a pointer from `tish_value_as_string`, freed once.
#[no_mangle]
pub unsafe extern "C" fn tish_string_free(s: *mut c_char) {
    if !s.is_null() {
        drop(CString::from_raw(s));
    }
}

// ── arrays ───────────────────────────────────────────────────────────────────
#[no_mangle]
pub extern "C" fn tish_value_array_new() -> TishValueRef {
    box_value(Value::Array(VmRef::new(Vec::new())))
}

/// Append a **clone** of `elem` to array `arr` (the caller keeps owning `elem`). No-op if `arr`
/// isn't an array.
/// # Safety: both must be null or live handles.
#[no_mangle]
pub unsafe extern "C" fn tish_value_array_push(arr: TishValueRef, elem: TishValueRef) {
    if let Some(Value::Array(a)) = as_value(arr) {
        let v = as_value(elem).cloned().unwrap_or(Value::Null);
        a.borrow_mut().push(v);
    }
}

/// Length of `arr`, or 0 if it isn't an array.
/// # Safety: `arr` must be null or a live handle.
#[no_mangle]
pub unsafe extern "C" fn tish_value_array_len(arr: TishValueRef) -> usize {
    match as_value(arr) {
        Some(Value::Array(a)) => a.borrow().len(),
        _ => 0,
    }
}

/// A newly-owned handle to a **clone** of element `i` (caller drops it); a null value handle if
/// out of range or not an array.
/// # Safety: `arr` must be null or a live handle.
#[no_mangle]
pub unsafe extern "C" fn tish_value_array_get(arr: TishValueRef, i: usize) -> TishValueRef {
    match as_value(arr) {
        Some(Value::Array(a)) => box_value(a.borrow().get(i).cloned().unwrap_or(Value::Null)),
        _ => box_value(Value::Null),
    }
}

// ── objects ──────────────────────────────────────────────────────────────────
#[no_mangle]
pub extern "C" fn tish_value_object_new() -> TishValueRef {
    box_value(Value::Object(VmRef::new(ObjectData::default())))
}

/// Set `obj[key]` to a **clone** of `val`. No-op if `obj` isn't an object or `key` is invalid.
/// # Safety: handles live; `key` a valid C string.
#[no_mangle]
pub unsafe extern "C" fn tish_value_object_set(
    obj: TishValueRef,
    key: *const c_char,
    val: TishValueRef,
) {
    if key.is_null() {
        return;
    }
    if let (Some(Value::Object(o)), Ok(k)) = (as_value(obj), CStr::from_ptr(key).to_str()) {
        let v = as_value(val).cloned().unwrap_or(Value::Null);
        o.borrow_mut().strings.insert(k.into(), v);
    }
}

/// Newly-owned handle to a **clone** of `obj[key]`; a null value handle if missing / not an object.
/// # Safety: `obj` live; `key` a valid C string.
#[no_mangle]
pub unsafe extern "C" fn tish_value_object_get(
    obj: TishValueRef,
    key: *const c_char,
) -> TishValueRef {
    if !key.is_null() {
        if let (Some(Value::Object(o)), Ok(k)) = (as_value(obj), CStr::from_ptr(key).to_str()) {
            return box_value(o.borrow().strings.get(k).cloned().unwrap_or(Value::Null));
        }
    }
    box_value(Value::Null)
}

// ── lifetime ─────────────────────────────────────────────────────────────────
/// Deep-share clone (`Value::clone` shares `Arc`/`Rc` containers, like the interpreter).
/// # Safety: `r` null or live.
#[no_mangle]
pub unsafe extern "C" fn tish_value_clone(r: TishValueRef) -> TishValueRef {
    box_value(as_value(r).cloned().unwrap_or(Value::Null))
}

/// Release a handle obtained from any `_new_*` / `_get` / `_clone` accessor.
/// # Safety: `r` null or a live handle, dropped exactly once.
#[no_mangle]
pub unsafe extern "C" fn tish_value_drop(r: TishValueRef) {
    if !r.is_null() {
        drop(Box::from_raw(r as *mut Value));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Helper: a value's tag + scalar round-trip through the C ABI, then drop.
    unsafe fn roundtrip_scalars() {
        let n = tish_value_new_number(42.5);
        assert_eq!(tish_value_tag(n), TISH_TAG_NUMBER);
        assert_eq!(tish_value_as_number(n), 42.5);
        tish_value_drop(n);

        let b = tish_value_new_bool(true);
        assert_eq!(tish_value_tag(b), TISH_TAG_BOOL);
        assert!(tish_value_as_bool(b));
        tish_value_drop(b);

        let z = tish_value_new_null();
        assert_eq!(tish_value_tag(z), TISH_TAG_NULL);
        tish_value_drop(z);

        // null pointer behaves as Null, never UB.
        assert_eq!(tish_value_tag(std::ptr::null_mut()), TISH_TAG_NULL);
        assert!(tish_value_as_number(std::ptr::null_mut()).is_nan());
    }

    #[test]
    fn scalars_roundtrip() {
        unsafe { roundtrip_scalars() }
    }

    #[test]
    fn string_roundtrip() {
        unsafe {
            let cs = CString::new("héllo").unwrap();
            let s = tish_value_new_string(cs.as_ptr());
            assert_eq!(tish_value_tag(s), TISH_TAG_STRING);
            let out = tish_value_as_string(s);
            assert!(!out.is_null());
            assert_eq!(CStr::from_ptr(out).to_str().unwrap(), "héllo");
            tish_string_free(out);
            // non-string → null pointer
            let n = tish_value_new_number(1.0);
            assert!(tish_value_as_string(n).is_null());
            tish_value_drop(n);
            tish_value_drop(s);
        }
    }

    #[test]
    fn array_roundtrip() {
        unsafe {
            let arr = tish_value_array_new();
            assert_eq!(tish_value_tag(arr), TISH_TAG_ARRAY);
            for i in 0..3 {
                let e = tish_value_new_number(i as f64 * 10.0);
                tish_value_array_push(arr, e);
                tish_value_drop(e); // push cloned; caller still owns e
            }
            assert_eq!(tish_value_array_len(arr), 3);
            let g = tish_value_array_get(arr, 1);
            assert_eq!(tish_value_as_number(g), 10.0);
            tish_value_drop(g);
            // out of range → null
            let oob = tish_value_array_get(arr, 9);
            assert_eq!(tish_value_tag(oob), TISH_TAG_NULL);
            tish_value_drop(oob);
            tish_value_drop(arr);
        }
    }

    #[test]
    fn object_roundtrip() {
        unsafe {
            let obj = tish_value_object_new();
            assert_eq!(tish_value_tag(obj), TISH_TAG_OBJECT);
            let key = CString::new("x").unwrap();
            let v = tish_value_new_number(7.0);
            tish_value_object_set(obj, key.as_ptr(), v);
            tish_value_drop(v);
            let got = tish_value_object_get(obj, key.as_ptr());
            assert_eq!(tish_value_as_number(got), 7.0);
            tish_value_drop(got);
            // missing key → null
            let miss = CString::new("nope").unwrap();
            let m = tish_value_object_get(obj, miss.as_ptr());
            assert_eq!(tish_value_tag(m), TISH_TAG_NULL);
            tish_value_drop(m);
            tish_value_drop(obj);
        }
    }

    // Simulates an extension fn `(a, b) => a + b` using ONLY the C ABI — the marshaling the B2
    // loader's shim will drive.
    extern "C" fn add_fn(args: *const TishValueRef, argc: usize) -> TishValueRef {
        unsafe {
            if argc < 2 {
                return tish_value_new_null();
            }
            let a = tish_value_as_number(*args);
            let b = tish_value_as_number(*args.add(1));
            tish_value_new_number(a + b)
        }
    }

    #[test]
    fn native_fn_call_shape() {
        unsafe {
            let a = tish_value_new_number(2.0);
            let b = tish_value_new_number(3.0);
            let argv: [TishValueRef; 2] = [a, b];
            let r = add_fn(argv.as_ptr(), 2);
            assert_eq!(tish_value_as_number(r), 5.0);
            tish_value_drop(r);
            tish_value_drop(a);
            tish_value_drop(b);
            // A module table referencing it type-checks (the `tish_module_register` shape).
            let name = CString::new("add").unwrap();
            let exports = [TishExport { name: name.as_ptr(), func: add_fn }];
            let table = TishExportTable { exports: exports.as_ptr(), count: 1 };
            assert_eq!(table.count, 1);
        }
    }
}
