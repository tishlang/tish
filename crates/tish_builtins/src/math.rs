//! Math builtin functions.

use crate::helpers::extract_num;
use tishlang_core::{to_int32, to_uint32, Value};

macro_rules! math_unary {
    ($name:ident, $op:ident) => {
        pub fn $name(args: &[Value]) -> Value {
            let n = extract_num(args.first()).unwrap_or(f64::NAN);
            Value::Number(n.$op())
        }
    };
}

math_unary!(abs, abs);
math_unary!(sqrt, sqrt);
math_unary!(floor, floor);
math_unary!(ceil, ceil);
math_unary!(sin, sin);
math_unary!(cos, cos);
math_unary!(tan, tan);
math_unary!(asin, asin);
math_unary!(acos, acos);
math_unary!(atan, atan);
math_unary!(log, ln);
math_unary!(log10, log10);
math_unary!(log2, log2);
math_unary!(exp, exp);
math_unary!(trunc, trunc);
math_unary!(cbrt, cbrt);
math_unary!(expm1, exp_m1);
math_unary!(log1p, ln_1p);

/// `Math.clz32(x)` — count leading zero bits of `ToUint32(x)` (0..=32).
pub fn clz32(args: &[Value]) -> Value {
    let n = to_uint32(extract_num(args.first()).unwrap_or(0.0));
    Value::Number(n.leading_zeros() as f64)
}

/// `Math.fround(x)` — round `x` to the nearest 32-bit float, back to f64.
pub fn fround(args: &[Value]) -> Value {
    let x = extract_num(args.first()).unwrap_or(f64::NAN);
    Value::Number(x as f32 as f64)
}

// --- f64-domain semantics (single source of truth) -------------------------------------------------
// These hold the JS-specific Math rules so every backend agrees: the vm/native paths call the
// `&[Value]` wrappers below, and the interpreter (which uses a different `Value` type) calls these f64
// helpers directly after extracting its own args. Keeping the rules here means they can't drift. #247

/// JS `Math.round`: ties round toward +∞ (not Rust's `.round()`, which rounds half away from zero),
/// and values in `[-0.5, 0)` return `-0`. `floor(n + 0.5)` gives the +∞-tie behavior; the guards keep
/// NaN/±∞ and the sign-of-zero edges correct (`Math.round(-2.5) === -2`, `Math.round(-0.5) === -0`).
pub fn round_f64(n: f64) -> f64 {
    if n.is_nan() || n.is_infinite() || n == 0.0 {
        n
    } else if (-0.5..0.5).contains(&n) {
        if n < 0.0 {
            -0.0
        } else {
            0.0
        }
    } else {
        (n + 0.5).floor()
    }
}

/// JS `Math.min` over already-extracted f64s: empty → `+∞`, any `NaN` → `NaN` (unlike `f64::min`,
/// which ignores NaN), and `-0` is preferred over `+0`.
pub fn min_f64(nums: &[f64]) -> f64 {
    let mut acc = f64::INFINITY;
    for &n in nums {
        if n.is_nan() {
            return f64::NAN;
        }
        if n < acc || (n == 0.0 && acc == 0.0 && n.is_sign_negative()) {
            acc = n;
        }
    }
    acc
}

/// JS `Math.max` over already-extracted f64s: empty → `-∞`, any `NaN` → `NaN`, `+0` preferred over `-0`.
pub fn max_f64(nums: &[f64]) -> f64 {
    let mut acc = f64::NEG_INFINITY;
    for &n in nums {
        if n.is_nan() {
            return f64::NAN;
        }
        if n > acc || (n == 0.0 && acc == 0.0 && n.is_sign_positive()) {
            acc = n;
        }
    }
    acc
}

pub fn round(args: &[Value]) -> Value {
    Value::Number(round_f64(extract_num(args.first()).unwrap_or(f64::NAN)))
}

pub fn min(args: &[Value]) -> Value {
    let nums: Vec<f64> = args
        .iter()
        .map(|v| extract_num(Some(v)).unwrap_or(f64::NAN))
        .collect();
    Value::Number(min_f64(&nums))
}

pub fn max(args: &[Value]) -> Value {
    let nums: Vec<f64> = args
        .iter()
        .map(|v| extract_num(Some(v)).unwrap_or(f64::NAN))
        .collect();
    Value::Number(max_f64(&nums))
}

pub fn pow(args: &[Value]) -> Value {
    let base = extract_num(args.first()).unwrap_or(f64::NAN);
    let exp = extract_num(args.get(1)).unwrap_or(f64::NAN);
    Value::Number(base.powf(exp))
}

pub fn random(_args: &[Value]) -> Value {
    Value::Number(rand::random::<f64>())
}

pub fn sign(args: &[Value]) -> Value {
    let n = extract_num(args.first()).unwrap_or(f64::NAN);
    Value::Number(if n.is_nan() {
        f64::NAN
    } else if n > 0.0 {
        1.0
    } else if n < 0.0 {
        -1.0
    } else {
        // ±0 → return the input unchanged so `Math.sign(-0)` is `-0` (per ES);
        // `n < 0.0` is false for -0, so it lands here — bare `0.0` would drop the sign.
        n
    })
}

pub fn atan2(args: &[Value]) -> Value {
    let y = extract_num(args.first()).unwrap_or(f64::NAN);
    let x = extract_num(args.get(1)).unwrap_or(f64::NAN);
    Value::Number(y.atan2(x))
}

/// ECMAScript `Math.hypot(...args)` core: sqrt of the sum of squares, VARIADIC and
/// NaN/Infinity-correct. Infinity takes precedence over NaN (any ±∞ arg → +∞, even alongside a NaN);
/// otherwise any NaN → NaN; the finite case is scaled by the max magnitude to avoid intermediate
/// overflow/underflow (matches V8). `Math.hypot()` (no args) → 0. Shared so interp/vm/native agree
/// (#247 — the interp/native path was 2-arg only, so `Math.hypot(3,4,12)` gave 5 not 13, and the vm
/// dropped non-number args instead of propagating NaN).
pub fn hypot_f64(nums: &[f64]) -> f64 {
    let mut any_nan = false;
    let mut max = 0.0_f64;
    for &n in nums {
        if n.is_infinite() {
            return f64::INFINITY;
        }
        if n.is_nan() {
            any_nan = true;
        }
        let a = n.abs();
        if a > max {
            max = a;
        }
    }
    if any_nan {
        return f64::NAN;
    }
    if max == 0.0 {
        return 0.0;
    }
    let mut sum = 0.0_f64;
    for &n in nums {
        let r = n / max;
        sum += r * r;
    }
    max * sum.sqrt()
}

pub fn hypot(args: &[Value]) -> Value {
    let nums: Vec<f64> = args
        .iter()
        .map(|v| extract_num(Some(v)).unwrap_or(f64::NAN))
        .collect();
    Value::Number(hypot_f64(&nums))
}

/// ES6 `Math.imul`: 32-bit integer multiply (used by xmur3 PRNG in juke-cards).
///
/// Operands go through JS `ToInt32` (modulo 2³², NaN/±Infinity → 0), NOT a saturating `as i32`
/// cast: the latter clamps any argument ≥ 2³¹ (e.g. a full uint32 hash state) to `i32::MAX` and
/// diverges from ECMAScript / V8 (`Math.imul(3e9, 1)` = -1294967296, not 2147483647). The typed
/// native path inlines the same ToInt32 + wrapping multiply, so all backends agree.
pub fn imul(args: &[Value]) -> Value {
    let a = to_int32(extract_num(args.first()).unwrap_or(0.0));
    let b = to_int32(extract_num(args.get(1)).unwrap_or(0.0));
    Value::Number(a.wrapping_mul(b) as f64)
}
