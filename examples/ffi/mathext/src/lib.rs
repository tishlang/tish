//! `mathext` — an example tish native extension (C-ABI cdylib).
//!
//! Demonstrates every shape the FFI marshals: numbers, strings, arrays (read), and objects
//! (build + return). Loaded from tish with `import { ... } from "ffi:./mathext/.../libmathext.dylib"`.
//! Each export is an `extern "C" fn(args: *const TishValueRef, argc) -> TishValueRef` that uses
//! only the host's `tish_value_*` accessors — it never sees tish's Rust `Value` type.

use std::ffi::{c_char, CStr, CString};

use tishlang_ffi::{
    tish_string_free, tish_value_array_get, tish_value_array_len, tish_value_as_number,
    tish_value_as_string, tish_value_new_number, tish_value_new_string, TishExport,
    TishExportTable, TishValueRef,
};

/// `hypot(a, b) = sqrt(a² + b²)` — two number args, number result.
extern "C" fn hypot(args: *const TishValueRef, argc: usize) -> TishValueRef {
    unsafe {
        if argc < 2 {
            return tish_value_new_number(f64::NAN);
        }
        let a = tish_value_as_number(*args);
        let b = tish_value_as_number(*args.add(1));
        tish_value_new_number((a * a + b * b).sqrt())
    }
}

/// `factorial(n)` — computed natively in Rust (the kind of hot compute you'd reach for FFI for).
extern "C" fn factorial(args: *const TishValueRef, argc: usize) -> TishValueRef {
    unsafe {
        if argc < 1 {
            return tish_value_new_number(f64::NAN);
        }
        let n = tish_value_as_number(*args).max(0.0) as u64;
        let mut f: f64 = 1.0;
        for i in 2..=n {
            f *= i as f64;
        }
        tish_value_new_number(f)
    }
}

/// `summary(array)` — reads an array arg, returns a `"sum=… mean=… max=… count=…"` string.
/// (A version returning an object `{ sum, mean, … }` works too, but only when this extension links
/// the *exact* same tish_core feature set as the host — object storage is an `IndexMap` whose layout
/// is feature-sensitive. The decoupled host-exported-accessor model removes that constraint; see
/// the README. Array reading + string building below are robust regardless.)
extern "C" fn summary(args: *const TishValueRef, argc: usize) -> TishValueRef {
    unsafe {
        let out = if argc < 1 {
            "sum=0 mean=0 max=0 count=0".to_string()
        } else {
            let arr = *args;
            let n = tish_value_array_len(arr);
            let mut sum = 0.0;
            let mut max = f64::NEG_INFINITY;
            for i in 0..n {
                let e = tish_value_array_get(arr, i);
                let v = tish_value_as_number(e);
                tishlang_ffi::tish_value_drop(e);
                sum += v;
                if v > max {
                    max = v;
                }
            }
            let mean = if n > 0 { sum / n as f64 } else { 0.0 };
            let max = if n > 0 { max } else { 0.0 };
            format!("sum={sum} mean={mean} max={max} count={n}")
        };
        let c = CString::new(out).unwrap();
        tish_value_new_string(c.as_ptr())
    }
}

/// `greet(name)` — string in, string out.
extern "C" fn greet(args: *const TishValueRef, argc: usize) -> TishValueRef {
    unsafe {
        let name = if argc >= 1 {
            let cs = tish_value_as_string(*args);
            if cs.is_null() {
                "world".to_string()
            } else {
                let s = CStr::from_ptr(cs).to_str().unwrap_or("world").to_string();
                tish_string_free(cs);
                s
            }
        } else {
            "world".to_string()
        };
        let out = CString::new(format!("Hello, {name}! (from native Rust)")).unwrap();
        tish_value_new_string(out.as_ptr())
    }
}

/// Module entry: a leaked static export table (built once, lives for the process).
#[no_mangle]
pub extern "C" fn tish_module_register() -> *const TishExportTable {
    let mk = |name: &str, func: extern "C" fn(*const TishValueRef, usize) -> TishValueRef| {
        TishExport {
            name: CString::new(name).unwrap().into_raw() as *const c_char,
            func,
        }
    };
    let exports = Box::leak(Box::new([
        mk("hypot", hypot),
        mk("factorial", factorial),
        mk("summary", summary),
        mk("greet", greet),
    ]));
    let table = Box::leak(Box::new(TishExportTable {
        exports: exports.as_ptr(),
        count: exports.len(),
    }));
    table as *const TishExportTable
}
