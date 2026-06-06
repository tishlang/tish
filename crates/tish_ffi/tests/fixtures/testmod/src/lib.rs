//! Fixture native extension (cdylib). Exposes `triple(x) = x*3` and `make_pair(a,b) = [a,b]`
//! through the C ABI, registered via `tish_module_register`. Loaded by the `load_module` test.

use std::ffi::{c_char, CString};

use tishlang_ffi::{
    tish_value_array_new, tish_value_array_push, tish_value_as_number, tish_value_new_null,
    tish_value_new_number, TishExport, TishExportTable, TishValueRef,
};

extern "C" fn triple(args: *const TishValueRef, argc: usize) -> TishValueRef {
    unsafe {
        if argc < 1 {
            return tish_value_new_null();
        }
        let x = tish_value_as_number(*args);
        tish_value_new_number(x * 3.0)
    }
}

extern "C" fn make_pair(args: *const TishValueRef, argc: usize) -> TishValueRef {
    unsafe {
        let arr = tish_value_array_new();
        for i in 0..argc.min(2) {
            tish_value_array_push(arr, *args.add(i));
        }
        arr
    }
}

/// Module entry point. Returns a leaked static table (process-lifetime, called once).
#[no_mangle]
pub extern "C" fn tish_module_register() -> *const TishExportTable {
    let mk = |name: &str, func: extern "C" fn(*const TishValueRef, usize) -> TishValueRef| {
        TishExport {
            name: CString::new(name).unwrap().into_raw() as *const c_char,
            func,
        }
    };
    let exports = Box::leak(Box::new([mk("triple", triple), mk("make_pair", make_pair)]));
    let table = Box::leak(Box::new(TishExportTable {
        exports: exports.as_ptr(),
        count: exports.len(),
    }));
    table as *const TishExportTable
}
