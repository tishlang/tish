//! Typed arrays (`Float32Array`, `Float64Array`, `Int8Array`, `Uint8Array`, `Uint8ClampedArray`,
//! `Int16Array`, `Uint16Array`, `Int32Array`, `Uint32Array`) for the non-JS targets.
//!
//! ## Representation
//! Unlike `Date`/`Set`/`Map`, typed arrays are array-LIKE — they need indexing (`ta[i]`), `.length`,
//! `for…of`, and the array methods. So a typed array is just a **`Value::Array`** (which gives all of
//! that for free across every backend), with each element **coerced to the view's element type at
//! construction**: `Float32Array` rounds to f32 precision, the integer views truncate-and-wrap
//! (`ToInt8`/`ToUint32`/…), and `Uint8ClampedArray` clamps to `0..=255` (round-half-to-even).
//!
//! ## v1 scope / gaps (documented in tishlang-web)
//! - **Element coercion happens on construction / `.from` / `.of`, not on element assignment.**
//!   `ta[i] = 300` stores `300` (a plain array write); rebuild via `Uint8Array.of(...)` if you need
//!   the wrap. (A packed-native representation enforces write-coercion via the Rust element type — a
//!   fast-follow.)
//! - No `ArrayBuffer` / `DataView` / `.buffer` / `.byteLength` / `.subarray` / `.set` yet.
//! - A typed array is a regular array at runtime, so `Array.isArray(ta)` is `true` and there is no
//!   `instanceof` distinction.

#[cfg(feature = "portable")]
#[allow(unused_imports)]
use alloc::{borrow::ToOwned, boxed::Box, format, string::{String, ToString}, vec, vec::Vec};
#[cfg(feature = "portable")]
use tishlang_core::FloatExt;

use tishlang_core::Arc;
use tishlang_core::{ObjectMap, Value, VmRef};

const CONSTRUCT: &str = "__construct";

/// Element type of a typed-array view.
#[derive(Clone, Copy)]
enum Kind {
    F64,
    F32,
    I8,
    U8,
    U8Clamped,
    I16,
    U16,
    I32,
    U32,
}

/// `ToUintN(x)` — ECMAScript truncate-then-wrap into `[0, 2^bits)`.
fn to_uint(x: f64, bits: u32) -> f64 {
    if !x.is_finite() {
        return 0.0;
    }
    let m = 2f64.powi(bits as i32);
    // `trunc` rounds toward zero (ToInteger); `rem_euclid` yields a non-negative remainder.
    x.trunc().rem_euclid(m)
}

/// `ToIntN(x)` — `ToUintN` reinterpreted as a signed `bits`-wide integer.
fn to_int(x: f64, bits: u32) -> f64 {
    let u = to_uint(x, bits);
    let half = 2f64.powi(bits as i32 - 1);
    if u >= half {
        u - 2.0 * half
    } else {
        u
    }
}

/// Coerce one number to the view's element type.
fn coerce(kind: Kind, x: f64) -> f64 {
    match kind {
        Kind::F64 => x,
        Kind::F32 => x as f32 as f64,
        Kind::U8Clamped => {
            if x.is_nan() || x <= 0.0 {
                0.0
            } else if x >= 255.0 {
                255.0
            } else {
                x.round_ties_even()
            }
        }
        Kind::I8 => to_int(x, 8),
        Kind::U8 => to_uint(x, 8),
        Kind::I16 => to_int(x, 16),
        Kind::U16 => to_uint(x, 16),
        Kind::I32 => to_int(x, 32),
        Kind::U32 => to_uint(x, 32),
    }
}

fn bytes_per_element(kind: Kind) -> f64 {
    match kind {
        Kind::F64 => 8.0,
        Kind::I32 | Kind::U32 | Kind::F32 => 4.0,
        Kind::I16 | Kind::U16 => 2.0,
        Kind::I8 | Kind::U8 | Kind::U8Clamped => 1.0,
    }
}

/// Array-like `value` → its elements (arrays and packed number-arrays; otherwise empty).
fn elements(value: &Value) -> Vec<Value> {
    match value {
        Value::Array(a) => a.borrow().clone(),
        Value::NumberArray(a) => a.borrow().to_values(),
        _ => Vec::new(),
    }
}

/// Build the backing `Value::Array`, coercing every element to `kind`. Non-numeric elements become
/// `NaN` first (→ `0` for the integer views).
fn from_values(kind: Kind, vals: &[Value]) -> Value {
    let out: Vec<Value> = vals
        .iter()
        .map(|v| Value::Number(coerce(kind, v.as_number().unwrap_or(f64::NAN))))
        .collect();
    Value::Array(VmRef::new(out))
}

/// `new X(...)`: a length `n` (zero-filled), or an array-like to copy+coerce, or empty.
fn construct(kind: Kind, args: &[Value]) -> Value {
    match args.first() {
        None => Value::Array(VmRef::new(Vec::new())),
        // `0` coerces to `0` for every element type, so a length-`n` view is `n` zeros.
        Some(Value::Number(n)) => {
            let len = n.max(0.0) as usize;
            Value::Array(VmRef::new(vec![Value::Number(0.0); len]))
        }
        Some(v) => from_values(kind, &elements(v)),
    }
}

/// Native-codegen-only packed constructor for `new Float64Array(...)`.
///
/// `Float64Array` is the one view whose element type *is* `f64`, so it needs no coercion and maps
/// exactly onto the packed [`Value::NumberArray`] (`Vec<f64>`) representation — eliminating the
/// per-element `Value` boxing the generic boxed `Value::Array` backing pays. When
/// [`Value::packed_arrays_enabled`] is **off** (the default), this returns the *identical* boxed
/// value the generic constructor would, so default builds stay byte-for-byte unchanged; the packed
/// form is only ever produced under `TISH_PACKED_ARRAYS=1`.
///
/// This lives behind the native codegen (interp/VM keep the boxed `Value::Array` — their value
/// bridges have no `NumberArray` variant), so on the native path a `NumberArray` is *always* a
/// `Float64Array`. That makes storing writes as `f64` the correct view semantics (and closes the
/// construction-only-coercion gap for this one view). Any op without a packed fast path materialises
/// it back to a boxed array, so every array method keeps working.
pub fn float64_array_packed(args: &[Value]) -> Value {
    float64_array_with(Value::packed_arrays_enabled(), args)
}

/// `float64_array_packed` with the packed/boxed choice made explicit, so tests exercise both paths
/// without toggling the now-cached process env flag (#166). `packed = false` is byte-identical to
/// the generic boxed `Value::Array` constructor.
fn float64_array_with(packed: bool, args: &[Value]) -> Value {
    if !packed {
        // Byte-identical fallback to the generic boxed `Value::Array` backing.
        return construct(Kind::F64, args);
    }
    let nums: Vec<f64> = match args.first() {
        None => Vec::new(),
        // Length form: `new Float64Array(n)` → `n` zeros (0.0 is the F64 coercion of 0).
        Some(Value::Number(n)) => vec![0.0; n.max(0.0) as usize],
        // Array-like copy: mirror `from_values(F64, …)` — non-numeric elements become `NaN`.
        Some(v) => elements(v)
            .iter()
            .map(|e| e.as_number().unwrap_or(f64::NAN))
            .collect(),
    };
    Value::number_array(nums)
}

/// The constructor object for one view kind: `__construct` + `from` / `of` / `BYTES_PER_ELEMENT`.
fn make_constructor(kind: Kind) -> Value {
    let mut m = ObjectMap::default();
    m.insert(
        Arc::from(CONSTRUCT),
        Value::native(move |args: &[Value]| construct(kind, args)),
    );
    m.insert(
        Arc::from("from"),
        Value::native(move |args: &[Value]| {
            from_values(kind, &args.first().map(elements).unwrap_or_default())
        }),
    );
    m.insert(
        Arc::from("of"),
        Value::native(move |args: &[Value]| from_values(kind, args)),
    );
    m.insert(
        Arc::from("BYTES_PER_ELEMENT"),
        Value::Number(bytes_per_element(kind)),
    );
    Value::object(m)
}

macro_rules! ctor_fn {
    ($name:ident, $kind:expr) => {
        pub fn $name() -> Value {
            make_constructor($kind)
        }
    };
}

ctor_fn!(float64_array_constructor_value, Kind::F64);
ctor_fn!(float32_array_constructor_value, Kind::F32);
ctor_fn!(int8_array_constructor_value, Kind::I8);
ctor_fn!(uint8_array_constructor_value, Kind::U8);
ctor_fn!(uint8_clamped_array_constructor_value, Kind::U8Clamped);
ctor_fn!(int16_array_constructor_value, Kind::I16);
ctor_fn!(uint16_array_constructor_value, Kind::U16);
ctor_fn!(int32_array_constructor_value, Kind::I32);
ctor_fn!(uint32_array_constructor_value, Kind::U32);

#[cfg(test)]
mod tests {
    use super::*;

    fn nums(v: &Value) -> Vec<f64> {
        match v {
            Value::Array(a) => a
                .borrow()
                .iter()
                .map(|e| match e {
                    Value::Number(n) => *n,
                    _ => f64::NAN,
                })
                .collect(),
            _ => vec![],
        }
    }

    #[test]
    fn float32_rounds_to_f32_precision() {
        // 1.1 is not representable in f32; the stored value is the f32-rounded double.
        let v = from_values(Kind::F32, &[Value::Number(1.1)]);
        assert_eq!(nums(&v)[0], 1.1f32 as f64);
        assert_ne!(nums(&v)[0], 1.1);
    }

    #[test]
    fn uint8_wraps() {
        let v = from_values(Kind::U8, &[Value::Number(300.0), Value::Number(-1.0), Value::Number(256.0)]);
        assert_eq!(nums(&v), vec![44.0, 255.0, 0.0]);
    }

    #[test]
    fn int8_wraps_signed() {
        let v = from_values(Kind::I8, &[Value::Number(127.0), Value::Number(128.0), Value::Number(-129.0)]);
        assert_eq!(nums(&v), vec![127.0, -128.0, 127.0]);
    }

    #[test]
    fn uint8_clamped_clamps_and_rounds_half_even() {
        let v = from_values(
            Kind::U8Clamped,
            &[Value::Number(-5.0), Value::Number(300.0), Value::Number(2.5), Value::Number(3.5)],
        );
        // 2.5 → 2 (round to even), 3.5 → 4 (round to even).
        assert_eq!(nums(&v), vec![0.0, 255.0, 2.0, 4.0]);
    }

    #[test]
    fn int_views_map_nan_to_zero() {
        let v = from_values(Kind::I32, &[Value::Null, Value::String("x".into())]);
        assert_eq!(nums(&v), vec![0.0, 0.0]);
    }

    #[test]
    fn construct_length_is_zero_filled() {
        let v = construct(Kind::U16, &[Value::Number(3.0)]);
        assert_eq!(nums(&v), vec![0.0, 0.0, 0.0]);
    }

    #[test]
    fn uint32_wraps_large() {
        let v = from_values(Kind::U32, &[Value::Number(4294967296.0), Value::Number(4294967297.0)]);
        assert_eq!(nums(&v), vec![0.0, 1.0]);
    }

    // The packed/boxed selection is passed explicitly via `float64_array_with`, so this exercises
    // both paths without touching the now-cached process env flag (#166) — no env mutation, no
    // mid-test toggle race, parallel-safe.
    #[test]
    fn float64_packed_respects_flag() {
        // Packed off (the default): byte-identical boxed `Value::Array` fallback.
        let boxed = float64_array_with(false, &[Value::Number(3.0)]);
        assert!(matches!(boxed, Value::Array(_)), "packed-off must return boxed Array");
        assert_eq!(nums(&boxed), vec![0.0, 0.0, 0.0]);

        // Packed on: `Value::NumberArray`. F64 needs no coercion (exact), non-numeric → NaN
        // (matching the boxed `from_values(F64, …)`), and the length form zero-fills.
        let packed = float64_array_with(
            true,
            &[Value::Array(VmRef::new(vec![
                Value::Number(1.1),
                Value::Number(2.2),
                Value::Null,
            ]))],
        );
        match &packed {
            Value::NumberArray(a) => {
                let v = a.borrow();
                let v = v.as_packed().expect("packed backing");
                assert_eq!(v[0], 1.1);
                assert_eq!(v[1], 2.2);
                assert!(v[2].is_nan());
            }
            _ => panic!("packed-on must return NumberArray"),
        }
        assert!(matches!(
            float64_array_with(true, &[Value::Number(2.0)]),
            Value::NumberArray(_)
        ));
    }
}
