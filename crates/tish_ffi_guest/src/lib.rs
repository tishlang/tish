//! # Module side of the tish FFI ABI v2
//!
//! A native module (cdylib) exports
//! `tish_module_register_v2(api: *const TishHostApi) -> *const TishExportTable`. It passes `api`
//! to [`install`], then each export trampoline calls [`export_call`], which deep-converts the
//! host's argument handles into the module's own [`Value`]s, runs the export, and converts the
//! result (or the export's throw) back.
//!
//! The module links its own `tishlang_core`; nothing here names a host symbol, so no linker flags
//! are needed on either side. Functions cross in both directions: a host function becomes a local
//! `Value::Function` that calls back through `api.call`, and a local function becomes a host
//! function via `api.new_function`. Calls happen on the calling thread.
//!
//! The `#[repr(C)]` types mirror `tishlang_ffi`'s host definitions field for field; the ABI is
//! the contract, and `abi_version` / `size` guard every field newer than v2.

use std::ffi::{c_char, c_void};
use std::sync::OnceLock;

use tishlang_core::{ObjectData, Value, VmRef};

pub type TishValueRef = *mut c_void;
pub type TishNativeFn = extern "C" fn(args: *const TishValueRef, argc: usize) -> TishValueRef;
pub type TishCallbackFn =
    extern "C" fn(ctx: *mut c_void, args: *const TishValueRef, argc: usize) -> TishValueRef;
pub type TishCtxDropFn = extern "C" fn(ctx: *mut c_void);

pub const TISH_FFI_ABI_VERSION: u32 = 2;
pub const TISH_TAG_NULL: i32 = 0;
pub const TISH_TAG_NUMBER: i32 = 1;
pub const TISH_TAG_STRING: i32 = 2;
pub const TISH_TAG_BOOL: i32 = 3;
pub const TISH_TAG_ARRAY: i32 = 4;
pub const TISH_TAG_OBJECT: i32 = 5;
pub const TISH_TAG_OTHER: i32 = 6;
pub const TISH_TAG_FUNCTION: i32 = 7;

#[repr(C)]
pub struct TishExport {
    pub name: *const c_char,
    pub func: TishNativeFn,
}

#[repr(C)]
pub struct TishExportTable {
    pub exports: *const TishExport,
    pub count: usize,
}

// SAFETY: the table only holds pointers to `'static` names and fn pointers.
unsafe impl Sync for TishExportTable {}
unsafe impl Send for TishExportTable {}

#[repr(C)]
pub struct TishHostApi {
    pub abi_version: u32,
    pub size: usize,
    pub new_null: extern "C" fn() -> TishValueRef,
    pub new_number: extern "C" fn(n: f64) -> TishValueRef,
    pub new_bool: extern "C" fn(b: bool) -> TishValueRef,
    pub new_string_utf8: unsafe extern "C" fn(ptr: *const u8, len: usize) -> TishValueRef,
    pub tag: unsafe extern "C" fn(r: TishValueRef) -> i32,
    pub as_number: unsafe extern "C" fn(r: TishValueRef) -> f64,
    pub as_bool: unsafe extern "C" fn(r: TishValueRef) -> bool,
    pub string_utf8: unsafe extern "C" fn(r: TishValueRef, len: *mut usize) -> *const u8,
    pub array_new: extern "C" fn() -> TishValueRef,
    pub array_push: unsafe extern "C" fn(arr: TishValueRef, elem: TishValueRef),
    pub array_len: unsafe extern "C" fn(arr: TishValueRef) -> usize,
    pub array_get: unsafe extern "C" fn(arr: TishValueRef, i: usize) -> TishValueRef,
    pub array_set: unsafe extern "C" fn(arr: TishValueRef, i: usize, elem: TishValueRef),
    pub object_new: extern "C" fn() -> TishValueRef,
    pub object_set_utf8:
        unsafe extern "C" fn(obj: TishValueRef, key: *const u8, key_len: usize, val: TishValueRef),
    pub object_get_utf8:
        unsafe extern "C" fn(obj: TishValueRef, key: *const u8, key_len: usize) -> TishValueRef,
    pub object_keys: unsafe extern "C" fn(obj: TishValueRef) -> TishValueRef,
    pub new_function: unsafe extern "C" fn(
        func: TishCallbackFn,
        ctx: *mut c_void,
        drop_ctx: Option<TishCtxDropFn>,
    ) -> TishValueRef,
    pub call:
        unsafe extern "C" fn(f: TishValueRef, args: *const TishValueRef, argc: usize) -> TishValueRef,
    pub throw_value: unsafe extern "C" fn(val: TishValueRef),
    pub take_throw: extern "C" fn() -> TishValueRef,
    pub clone: unsafe extern "C" fn(r: TishValueRef) -> TishValueRef,
    pub drop: unsafe extern "C" fn(r: TishValueRef),
}

static API: OnceLock<&'static TishHostApi> = OnceLock::new();

/// Record the host's table. Returns `false` (and the module should return a null export table)
/// if the host is older than v2 or its table is smaller than this module expects.
///
/// # Safety
/// `api` must be null or point to a host table that lives for the rest of the process.
pub unsafe fn install(api: *const TishHostApi) -> bool {
    let Some(api) = api.as_ref() else {
        return false;
    };
    if api.abi_version < TISH_FFI_ABI_VERSION || api.size < std::mem::size_of::<TishHostApi>() {
        return false;
    }
    let _ = API.set(api);
    true
}

/// The installed host table. Panics if called before [`install`] succeeded.
pub fn api() -> &'static TishHostApi {
    API.get().expect("tish ffi guest: host api not installed")
}

/// Build a leaked export table for `tish_module_register_v2` to return.
pub fn export_table(exports: &[(&'static [u8], TishNativeFn)]) -> *const TishExportTable {
    let entries: Vec<TishExport> = exports
        .iter()
        .map(|(name, func)| TishExport {
            name: name.as_ptr() as *const c_char,
            func: *func,
        })
        .collect();
    let entries = Box::leak(entries.into_boxed_slice());
    Box::leak(Box::new(TishExportTable {
        exports: entries.as_ptr(),
        count: entries.len(),
    }))
}

/// Run one export: convert args, call `f`, and convert its result. If the export threw (as
/// reported by `last_throw`, the emitted module's `tish_last_throw`), the throw is raised in the
/// host and a null handle is returned.
///
/// # Safety
/// `args` must point to `argc` live host handles; [`install`] must have succeeded.
pub unsafe fn export_call(
    args: *const TishValueRef,
    argc: usize,
    f: impl FnOnce(Vec<Value>) -> Value,
    last_throw: impl FnOnce() -> Value,
) -> TishValueRef {
    let argv: Vec<Value> = if args.is_null() {
        Vec::new()
    } else {
        std::slice::from_raw_parts(args, argc)
            .iter()
            .map(|h| from_host(*h))
            .collect()
    };
    let out = f(argv);
    let thrown = last_throw();
    if !matches!(thrown, Value::Null) {
        raise_in_host(&thrown);
        return (api().new_null)();
    }
    to_host(&out)
}

unsafe fn raise_in_host(thrown: &Value) {
    let api = api();
    let h = to_host(thrown);
    (api.throw_value)(h);
    (api.drop)(h);
}

/// Host handle → module value (deep copy; functions become callbacks into the host).
///
/// # Safety
/// `h` must be null or a live host handle; [`install`] must have succeeded.
pub unsafe fn from_host(h: TishValueRef) -> Value {
    let api = api();
    match (api.tag)(h) {
        TISH_TAG_NUMBER => Value::Number((api.as_number)(h)),
        TISH_TAG_BOOL => Value::Bool((api.as_bool)(h)),
        TISH_TAG_STRING => {
            let mut len = 0usize;
            let p = (api.string_utf8)(h, &mut len);
            if p.is_null() {
                return Value::Null;
            }
            let bytes = std::slice::from_raw_parts(p, len);
            Value::String(String::from_utf8_lossy(bytes).as_ref().into())
        }
        TISH_TAG_ARRAY => {
            let n = (api.array_len)(h);
            let mut out = Vec::with_capacity(n);
            for i in 0..n {
                let e = (api.array_get)(h, i);
                out.push(from_host(e));
                (api.drop)(e);
            }
            Value::Array(VmRef::new(out))
        }
        TISH_TAG_OBJECT => {
            let keys = (api.object_keys)(h);
            let n = (api.array_len)(keys);
            let mut obj = ObjectData::default();
            for i in 0..n {
                let k = (api.array_get)(keys, i);
                if let Value::String(key) = from_host(k) {
                    let b = key.as_bytes();
                    let v = (api.object_get_utf8)(h, b.as_ptr(), b.len());
                    obj.strings.insert(key.as_str().into(), from_host(v));
                    (api.drop)(v);
                }
                (api.drop)(k);
            }
            (api.drop)(keys);
            Value::Object(VmRef::new(obj))
        }
        TISH_TAG_FUNCTION => host_function(h),
        _ => Value::Null,
    }
}

/// A retained host function handle, released when the wrapping local function drops.
struct HostFn(TishValueRef);

// SAFETY: the handle is only dereferenced by the host through the api table, on the calling
// thread; the module never shares it across threads concurrently (calls are thread-affine).
unsafe impl Send for HostFn {}
unsafe impl Sync for HostFn {}

impl Drop for HostFn {
    fn drop(&mut self) {
        unsafe { (api().drop)(self.0) }
    }
}

unsafe fn host_function(h: TishValueRef) -> Value {
    let held = HostFn((api().clone)(h));
    Value::native(move |args: &[Value]| unsafe {
        // Borrow the whole guard: capturing only `held.0` would drop (and release) it early.
        let held = &held;
        let api = api();
        let handles: Vec<TishValueRef> = args.iter().map(|a| to_host(a)).collect();
        let r = (api.call)(held.0, handles.as_ptr(), handles.len());
        for h in handles {
            (api.drop)(h);
        }
        let thrown = (api.take_throw)();
        if !thrown.is_null() {
            let e = from_host(thrown);
            (api.drop)(thrown);
            (api.drop)(r);
            tishlang_core::set_pending_throw(e);
            return Value::Null;
        }
        let out = from_host(r);
        (api.drop)(r);
        out
    })
}

/// Module value → freshly owned host handle (deep copy; functions become host callables).
///
/// # Safety
/// [`install`] must have succeeded.
pub unsafe fn to_host(v: &Value) -> TishValueRef {
    let api = api();
    match v {
        Value::Null => (api.new_null)(),
        Value::Number(n) => (api.new_number)(*n),
        Value::Bool(b) => (api.new_bool)(*b),
        Value::String(s) => {
            let b = s.as_bytes();
            (api.new_string_utf8)(b.as_ptr(), b.len())
        }
        Value::Array(a) => {
            let arr = (api.array_new)();
            for e in a.borrow().iter() {
                let h = to_host(e);
                (api.array_push)(arr, h);
                (api.drop)(h);
            }
            arr
        }
        Value::NumberArray(a) => {
            let arr = (api.array_new)();
            for e in a.borrow().to_values() {
                let h = to_host(&e);
                (api.array_push)(arr, h);
                (api.drop)(h);
            }
            arr
        }
        Value::Object(o) => {
            let obj = (api.object_new)();
            for (k, val) in o.borrow().strings.iter() {
                let h = to_host(val);
                let kb = k.as_bytes();
                (api.object_set_utf8)(obj, kb.as_ptr(), kb.len(), h);
                (api.drop)(h);
            }
            obj
        }
        Value::Function(_) => {
            let ctx = Box::into_raw(Box::new(v.clone())) as *mut c_void;
            (api.new_function)(local_fn_trampoline, ctx, Some(local_fn_drop))
        }
        _ => (api.new_null)(),
    }
}

extern "C" fn local_fn_trampoline(
    ctx: *mut c_void,
    args: *const TishValueRef,
    argc: usize,
) -> TishValueRef {
    unsafe {
        let Value::Function(f) = &*(ctx as *const Value) else {
            return (api().new_null)();
        };
        let argv: Vec<Value> = if args.is_null() {
            Vec::new()
        } else {
            std::slice::from_raw_parts(args, argc)
                .iter()
                .map(|h| from_host(*h))
                .collect()
        };
        let _ = tishlang_core::take_pending_throw();
        let out = f.call(&argv);
        if let Some(thrown) = tishlang_core::take_pending_throw() {
            raise_in_host(&thrown);
            return (api().new_null)();
        }
        to_host(&out)
    }
}

extern "C" fn local_fn_drop(ctx: *mut c_void) {
    unsafe { drop(Box::from_raw(ctx as *mut Value)) }
}
