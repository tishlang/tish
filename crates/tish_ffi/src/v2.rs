//! # ABI v2 — host accessors delivered as a function-pointer table
//!
//! v1 extensions resolve the host's `tish_value_*` symbols at load time, which needs the host
//! binary to export them (`-export_dynamic`) and keep them alive under LTO + strip. v2 hands the
//! module a [`TishHostApi`] table instead: the loader calls
//! `tish_module_register_v2(api: *const TishHostApi) -> *const TishExportTable`, the module keeps
//! the pointer, and every host access goes through it. No symbol export, no late binding, and
//! taking the functions' addresses keeps them alive in any host profile.
//!
//! Additions over v1: length-delimited UTF-8 strings (interior NUL allowed), object key
//! enumeration, array set, function handles in both directions ([`TishHostApi::new_function`] /
//! [`TishHostApi::call`]), and a throw channel ([`TishHostApi::throw_value`] /
//! [`TishHostApi::take_throw`]). Packed `NumberArray` values read as arrays.
//!
//! Ownership follows v1: handles returned by `new_*` / `*_get` / `object_keys` / `call` /
//! `take_throw` / `clone` are owned by the caller and released with `drop`; `*_push` / `*_set` /
//! `throw_value` clone their arguments. [`TishHostApi::string_utf8`] returns a pointer *borrowed*
//! from the handle, valid until that handle is dropped.
//!
//! Evolution: fields are only ever appended. A module checks `abi_version` and `size` before
//! touching a field newer than the version it was built for.

use std::cell::RefCell;
use std::ffi::c_void;
use std::sync::Arc;

use tishlang_core::{ObjectData, Value, VmRef};

use crate::{
    box_value, tish_value_array_new, tish_value_array_push, tish_value_clone, tish_value_drop,
    tish_value_new_bool, tish_value_new_null, tish_value_new_number, tish_value_object_new,
    TishValueRef, TISH_TAG_ARRAY, TISH_TAG_BOOL, TISH_TAG_NULL, TISH_TAG_NUMBER, TISH_TAG_OBJECT,
    TISH_TAG_OTHER, TISH_TAG_STRING,
};

pub const TISH_FFI_ABI_VERSION: u32 = 2;

/// v2-only tag: a callable value (pass it to [`TishHostApi::call`]). v1's `tag` keeps reporting
/// functions as `TISH_TAG_OTHER`.
pub const TISH_TAG_FUNCTION: i32 = 7;

/// A foreign callback wrapped by [`TishHostApi::new_function`]: borrowed argument handles in,
/// freshly-owned result handle out (same contract as `TishNativeFn`, plus a context pointer).
pub type TishCallbackFn =
    extern "C" fn(ctx: *mut c_void, args: *const TishValueRef, argc: usize) -> TishValueRef;

/// Releases a callback context when the last reference to the wrapping function value drops.
pub type TishCtxDropFn = extern "C" fn(ctx: *mut c_void);

/// The host API table. `#[repr(C)]`; fields are append-only across versions.
#[repr(C)]
pub struct TishHostApi {
    pub abi_version: u32,
    /// `size_of::<TishHostApi>()` as built by the host.
    pub size: usize,

    pub new_null: extern "C" fn() -> TishValueRef,
    pub new_number: extern "C" fn(n: f64) -> TishValueRef,
    pub new_bool: extern "C" fn(b: bool) -> TishValueRef,
    pub new_string_utf8: unsafe extern "C" fn(ptr: *const u8, len: usize) -> TishValueRef,

    /// `TISH_TAG_*`, with arrays covering `NumberArray` and [`TISH_TAG_FUNCTION`] for callables.
    pub tag: unsafe extern "C" fn(r: TishValueRef) -> i32,
    pub as_number: unsafe extern "C" fn(r: TishValueRef) -> f64,
    pub as_bool: unsafe extern "C" fn(r: TishValueRef) -> bool,
    /// Borrowed UTF-8 bytes of a string handle (null if not a string); `*len` receives the length.
    pub string_utf8: unsafe extern "C" fn(r: TishValueRef, len: *mut usize) -> *const u8,

    pub array_new: extern "C" fn() -> TishValueRef,
    pub array_push: unsafe extern "C" fn(arr: TishValueRef, elem: TishValueRef),
    pub array_len: unsafe extern "C" fn(arr: TishValueRef) -> usize,
    pub array_get: unsafe extern "C" fn(arr: TishValueRef, i: usize) -> TishValueRef,
    /// Sets element `i`, padding with null when `i >= len`.
    pub array_set: unsafe extern "C" fn(arr: TishValueRef, i: usize, elem: TishValueRef),

    pub object_new: extern "C" fn() -> TishValueRef,
    pub object_set_utf8:
        unsafe extern "C" fn(obj: TishValueRef, key: *const u8, key_len: usize, val: TishValueRef),
    pub object_get_utf8:
        unsafe extern "C" fn(obj: TishValueRef, key: *const u8, key_len: usize) -> TishValueRef,
    /// Array of the object's own string keys, in insertion order.
    pub object_keys: unsafe extern "C" fn(obj: TishValueRef) -> TishValueRef,

    /// Wrap a foreign callback as a host function value. `drop_ctx` (nullable) runs once when the
    /// function value is released.
    pub new_function: unsafe extern "C" fn(
        func: TishCallbackFn,
        ctx: *mut c_void,
        drop_ctx: Option<TishCtxDropFn>,
    ) -> TishValueRef,
    /// Call a function handle. If it throws, returns a null handle and parks the thrown value for
    /// [`TishHostApi::take_throw`] on this thread.
    pub call:
        unsafe extern "C" fn(f: TishValueRef, args: *const TishValueRef, argc: usize) -> TishValueRef,
    /// Raise `val` in the host when the current native call returns.
    pub throw_value: unsafe extern "C" fn(val: TishValueRef),
    /// The value thrown by the last failing [`TishHostApi::call`] on this thread, or a null
    /// pointer if none. Clears it.
    pub take_throw: extern "C" fn() -> TishValueRef,

    pub clone: unsafe extern "C" fn(r: TishValueRef) -> TishValueRef,
    pub drop: unsafe extern "C" fn(r: TishValueRef),
}

/// The host's table, handed to every v2 module the loader registers.
pub static TISH_HOST_API: TishHostApi = TishHostApi {
    abi_version: TISH_FFI_ABI_VERSION,
    size: std::mem::size_of::<TishHostApi>(),
    new_null: tish_value_new_null,
    new_number: tish_value_new_number,
    new_bool: tish_value_new_bool,
    new_string_utf8: v2_new_string_utf8,
    tag: v2_tag,
    as_number: v2_as_number,
    as_bool: v2_as_bool,
    string_utf8: v2_string_utf8,
    array_new: tish_value_array_new,
    array_push: tish_value_array_push,
    array_len: v2_array_len,
    array_get: v2_array_get,
    array_set: v2_array_set,
    object_new: tish_value_object_new,
    object_set_utf8: v2_object_set_utf8,
    object_get_utf8: v2_object_get_utf8,
    object_keys: v2_object_keys,
    new_function: v2_new_function,
    call: v2_call,
    throw_value: v2_throw_value,
    take_throw: v2_take_throw,
    clone: tish_value_clone,
    drop: tish_value_drop,
};

thread_local! {
    static LAST_CALL_THROW: RefCell<Option<Value>> = const { RefCell::new(None) };
}

#[inline]
unsafe fn val<'a>(r: TishValueRef) -> Option<&'a Value> {
    (r as *const Value).as_ref()
}

#[inline]
unsafe fn utf8<'a>(ptr: *const u8, len: usize) -> Option<&'a str> {
    if ptr.is_null() {
        return if len == 0 { Some("") } else { None };
    }
    std::str::from_utf8(std::slice::from_raw_parts(ptr, len)).ok()
}

fn object_of(v: &Value) -> Option<VmRef<ObjectData>> {
    match v {
        Value::Object(o) => Some(o.clone()),
        _ => None,
    }
}

unsafe extern "C" fn v2_new_string_utf8(ptr: *const u8, len: usize) -> TishValueRef {
    match utf8(ptr, len) {
        Some(s) => box_value(Value::String(s.into())),
        None => box_value(Value::Null),
    }
}

unsafe extern "C" fn v2_tag(r: TishValueRef) -> i32 {
    match val(r) {
        None | Some(Value::Null) => TISH_TAG_NULL,
        Some(Value::Number(_)) => TISH_TAG_NUMBER,
        Some(Value::String(_)) => TISH_TAG_STRING,
        Some(Value::Bool(_)) => TISH_TAG_BOOL,
        Some(Value::Array(_)) | Some(Value::NumberArray(_)) => TISH_TAG_ARRAY,
        Some(Value::Object(_)) => TISH_TAG_OBJECT,
        Some(Value::Function(_)) => TISH_TAG_FUNCTION,
        Some(_) => TISH_TAG_OTHER,
    }
}

unsafe extern "C" fn v2_as_number(r: TishValueRef) -> f64 {
    match val(r) {
        Some(Value::Number(n)) => *n,
        _ => f64::NAN,
    }
}

unsafe extern "C" fn v2_as_bool(r: TishValueRef) -> bool {
    matches!(val(r), Some(Value::Bool(true)))
}

unsafe extern "C" fn v2_string_utf8(r: TishValueRef, len: *mut usize) -> *const u8 {
    let (ptr, n) = match val(r) {
        Some(Value::String(s)) => (s.as_bytes().as_ptr(), s.as_bytes().len()),
        _ => (std::ptr::null(), 0),
    };
    if !len.is_null() {
        *len = n;
    }
    ptr
}

unsafe extern "C" fn v2_array_len(arr: TishValueRef) -> usize {
    match val(arr) {
        Some(Value::Array(a)) => a.borrow().len(),
        Some(Value::NumberArray(a)) => a.borrow().len(),
        _ => 0,
    }
}

unsafe extern "C" fn v2_array_get(arr: TishValueRef, i: usize) -> TishValueRef {
    let v = match val(arr) {
        Some(Value::Array(a)) => a.borrow().get(i).cloned(),
        Some(Value::NumberArray(a)) => a.borrow().get(i),
        _ => None,
    };
    box_value(v.unwrap_or(Value::Null))
}

unsafe extern "C" fn v2_array_set(arr: TishValueRef, i: usize, elem: TishValueRef) {
    let v = val(elem).cloned().unwrap_or(Value::Null);
    match val(arr) {
        Some(Value::Array(a)) => {
            let mut a = a.borrow_mut();
            if i >= a.len() {
                a.resize(i + 1, Value::Null);
            }
            a[i] = v;
        }
        Some(Value::NumberArray(a)) => {
            let mut a = a.borrow_mut();
            let mut vals = a.to_values();
            if i >= vals.len() {
                vals.resize(i + 1, Value::Null);
            }
            vals[i] = v;
            *a = tishlang_core::NumArrayBacking::Boxed(vals);
        }
        _ => {}
    }
}

unsafe extern "C" fn v2_object_set_utf8(
    obj: TishValueRef,
    key: *const u8,
    key_len: usize,
    v: TishValueRef,
) {
    if let (Some(Value::Object(o)), Some(k)) = (val(obj), utf8(key, key_len)) {
        let v = val(v).cloned().unwrap_or(Value::Null);
        o.borrow_mut().strings.insert(k.into(), v);
    }
}

unsafe extern "C" fn v2_object_get_utf8(
    obj: TishValueRef,
    key: *const u8,
    key_len: usize,
) -> TishValueRef {
    let got = match (val(obj).and_then(object_of), utf8(key, key_len)) {
        (Some(o), Some(k)) => o.borrow().strings.get(k).cloned(),
        _ => None,
    };
    box_value(got.unwrap_or(Value::Null))
}

unsafe extern "C" fn v2_object_keys(obj: TishValueRef) -> TishValueRef {
    let keys: Vec<Value> = match val(obj).and_then(object_of) {
        Some(o) => o
            .borrow()
            .strings
            .keys()
            .map(|k| Value::String(k.as_ref().into()))
            .collect(),
        None => Vec::new(),
    };
    box_value(Value::Array(VmRef::new(keys)))
}

/// A foreign callback + context, owned by the function value wrapping it.
struct ForeignCallback {
    func: TishCallbackFn,
    ctx: *mut c_void,
    drop_ctx: Option<TishCtxDropFn>,
}

// SAFETY: the context is opaque to the host; the module that created it owns its thread-safety
// (the documented contract is that callbacks are invoked on the thread that calls them).
unsafe impl Send for ForeignCallback {}
unsafe impl Sync for ForeignCallback {}

impl Drop for ForeignCallback {
    fn drop(&mut self) {
        if let Some(d) = self.drop_ctx {
            d(self.ctx);
        }
    }
}

unsafe extern "C" fn v2_new_function(
    func: TishCallbackFn,
    ctx: *mut c_void,
    drop_ctx: Option<TishCtxDropFn>,
) -> TishValueRef {
    let cb = Arc::new(ForeignCallback {
        func,
        ctx,
        drop_ctx,
    });
    box_value(Value::native(move |args: &[Value]| {
        let cb = &cb;
        crate::call_through_handles(args, |argv, argc| (cb.func)(cb.ctx, argv, argc))
    }))
}

unsafe extern "C" fn v2_call(
    f: TishValueRef,
    args: *const TishValueRef,
    argc: usize,
) -> TishValueRef {
    let Some(Value::Function(func)) = val(f) else {
        LAST_CALL_THROW.with(|t| {
            *t.borrow_mut() = Some(Value::String("ffi: call of non-function".into()))
        });
        return box_value(Value::Null);
    };
    let argv: Vec<Value> = if args.is_null() {
        Vec::new()
    } else {
        std::slice::from_raw_parts(args, argc)
            .iter()
            .map(|h| val(*h).cloned().unwrap_or(Value::Null))
            .collect()
    };
    let out = func.call(&argv);
    if let Some(thrown) = tishlang_core::take_pending_throw() {
        LAST_CALL_THROW.with(|t| *t.borrow_mut() = Some(thrown));
        return box_value(Value::Null);
    }
    box_value(out)
}

unsafe extern "C" fn v2_throw_value(v: TishValueRef) {
    tishlang_core::set_pending_throw(val(v).cloned().unwrap_or(Value::Null));
}

extern "C" fn v2_take_throw() -> TishValueRef {
    match LAST_CALL_THROW.with(|t| t.borrow_mut().take()) {
        Some(v) => box_value(v),
        None => std::ptr::null_mut(),
    }
}
