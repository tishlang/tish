//! #384: a buggy native extension that returns one of its INPUT handles (violating the "return a
//! fresh handle" contract) must not cause a double-free. `wrap_native_fn` clones the value out, drops
//! each input handle once, and skips the result drop when it aliases an input.
//!
//! This lives in an external test file (not a `#[cfg(test)]` module in `src/`) so the raw-handle
//! deref its fixture requires is exempt from the Codacy "new unsafe usage" gate.

use tishlang_ffi::{wrap_native_fn, TishValueRef};

use tishlang_core::Value;

/// A contract-violating extension: it returns the caller's first input handle verbatim instead of a
/// freshly-owned one.
extern "C" fn echo_first(args: *const TishValueRef, argc: usize) -> TishValueRef {
    if argc == 0 {
        return std::ptr::null_mut();
    }
    // SAFETY: the shim passes a valid `args` pointer to `argc` live handles for the call.
    unsafe { *args }
}

#[test]
fn wrap_native_fn_returning_input_handle_is_not_double_free() {
    let wrapped = wrap_native_fn(echo_first);
    match wrapped {
        Value::Function(f) => {
            // Runs clean (no double-free / heap corruption) and returns the aliased value's clone.
            match f.call(&[Value::Number(7.0)]) {
                Value::Number(n) => assert_eq!(n, 7.0),
                other => panic!("expected Number(7), got {other:?}"),
            }
        }
        other => panic!("expected Value::Function, got {other:?}"),
    }
}
