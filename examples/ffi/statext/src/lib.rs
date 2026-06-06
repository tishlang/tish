//! `statext` — a DECOUPLED tish native extension (cdylib).
//!
//! It links nothing tish-related. The C ABI is declared locally and the `tish_value_*` accessors are
//! `extern "C"` imports resolved against the host at dlopen. Because all values are created and read
//! through the host's accessors, there is a single `tish_core` — so this extension can build + return
//! an **object** with no layout/feature/dep matching (the case the linked model couldn't do).

use std::ffi::CString;
use std::os::raw::{c_char, c_void};

/// Opaque handle (matches the host's `TishValueRef`).
type TishValueRef = *mut c_void;

#[repr(C)]
pub struct TishExport {
    pub name: *const c_char,
    pub func: extern "C" fn(*const TishValueRef, usize) -> TishValueRef,
}

#[repr(C)]
pub struct TishExportTable {
    pub exports: *const TishExport,
    pub count: usize,
}

// The host's accessors — resolved at load time, never linked.
extern "C" {
    fn tish_value_new_number(n: f64) -> TishValueRef;
    fn tish_value_as_number(r: TishValueRef) -> f64;
    fn tish_value_array_len(arr: TishValueRef) -> usize;
    fn tish_value_array_get(arr: TishValueRef, i: usize) -> TishValueRef;
    fn tish_value_object_new() -> TishValueRef;
    fn tish_value_object_set(obj: TishValueRef, key: *const c_char, val: TishValueRef);
    fn tish_value_drop(r: TishValueRef);
}

/// `stats(array)` → object `{ sum, mean, max, count }` — array in, OBJECT out.
extern "C" fn stats(args: *const TishValueRef, argc: usize) -> TishValueRef {
    unsafe {
        let obj = tish_value_object_new();
        let set = |key: &str, val: f64| {
            let k = CString::new(key).unwrap();
            let v = tish_value_new_number(val);
            tish_value_object_set(obj, k.as_ptr(), v);
            tish_value_drop(v);
        };
        if argc < 1 {
            set("sum", 0.0);
            set("mean", 0.0);
            set("max", 0.0);
            set("count", 0.0);
            return obj;
        }
        let arr = *args;
        let n = tish_value_array_len(arr);
        let mut sum = 0.0;
        let mut max = f64::NEG_INFINITY;
        for i in 0..n {
            let e = tish_value_array_get(arr, i);
            sum += tish_value_as_number(e);
            let v = tish_value_as_number(e);
            if v > max {
                max = v;
            }
            tish_value_drop(e);
        }
        set("sum", sum);
        set("mean", if n > 0 { sum / n as f64 } else { 0.0 });
        set("max", if n > 0 { max } else { 0.0 });
        set("count", n as f64);
        obj
    }
}

#[no_mangle]
pub extern "C" fn tish_module_register() -> *const TishExportTable {
    let name = CString::new("stats").unwrap().into_raw() as *const c_char;
    let exports = Box::leak(Box::new([TishExport { name, func: stats }]));
    let table = Box::leak(Box::new(TishExportTable {
        exports: exports.as_ptr(),
        count: 1,
    }));
    table as *const TishExportTable
}
