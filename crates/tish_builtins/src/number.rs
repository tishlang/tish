//! Number builtin methods.
//!
//! Canonical, backend-agnostic implementations of `Number.prototype` methods.
//! The VM (`get_member`), the Rust runtime (`tishlang_runtime::number_to_fixed`),
//! and the tree-walk interpreter all route through here so every backend produces
//! byte-identical output — see `tish/docs/full-backend-parity-plan.md` (Workstream A).

use tishlang_core::Value;

/// `Number.prototype.toFixed(digits)` — ECMA-262 §21.1.3.3.
///
/// Formats the number using fixed-point notation with `digits` fraction digits.
/// `digits` is clamped to 0–20 (ECMA range) and defaults to 0 when absent/non-numeric,
/// matching `(1.5).toFixed() === "2"`. A non-number receiver yields `"NaN"`.
pub fn to_fixed(n: &Value, digits: &Value) -> Value {
    let num = match n {
        Value::Number(x) => *x,
        _ => f64::NAN,
    };
    let d = match digits {
        Value::Number(x) => (*x as i32).clamp(0, 20),
        _ => 0,
    } as usize;
    Value::String(to_fixed_str(num, d).into())
}

/// f64-domain `Number.prototype.toFixed` so the tree-walk interpreter (distinct `Value`) shares the
/// exact behavior. Rust's `{:.*}` rounds half-to-even (`(2.5).toFixed(0)` → "2"); JS rounds half away
/// from zero (→ "3"), so pre-round the scaled value with `.round()` (half-away) before formatting. #247
pub fn to_fixed_str(num: f64, digits: usize) -> String {
    if num.is_nan() {
        return "NaN".to_string();
    }
    if num.is_infinite() {
        return if num < 0.0 { "-Infinity" } else { "Infinity" }.to_string();
    }
    // JS switches to exponential form for |num| >= 1e21; defer to default formatting there.
    if num.abs() >= 1e21 {
        return format!("{}", num);
    }
    let factor = 10f64.powi(digits as i32);
    let rounded = (num * factor).round() / factor;
    format!("{:.*}", digits, rounded)
}

/// `Number.prototype.toString([radix])` — ECMA-262 §21.1.3.6.
///
/// Radix defaults to 10 (canonical JS number formatting). For radix 2–36 the value is
/// rendered in that base: sign, integer part via repeated division, and a fractional part
/// (bounded to 52 digits, like V8). NaN / ±Infinity stringify as in base 10 regardless of
/// radix. An out-of-range radix yields `"RadixError"` so the caller can surface a RangeError.
pub fn to_string(n: &Value, radix: &Value) -> Value {
    let num = match n {
        Value::Number(x) => *x,
        _ => f64::NAN,
    };
    let r = match radix {
        Value::Number(x) => *x as i64,
        _ => 10,
    };
    match number_to_string_radix(num, r) {
        Some(s) => Value::String(s.into()),
        None => Value::String("RadixError".into()),
    }
}

/// Backend-agnostic core of `Number.prototype.toString`: works on a plain `f64` so the
/// tree-walk interpreter (whose `Value` is a distinct type) can share the exact same
/// formatting. Returns `None` for an out-of-range radix (caller surfaces a RangeError).
pub fn number_to_string_radix(num: f64, radix: i64) -> Option<String> {
    if !(2..=36).contains(&radix) {
        return None;
    }
    if radix == 10 || num.is_nan() || num.is_infinite() {
        return Some(tishlang_core::js_number_to_string(num));
    }
    let radix = radix as u32;
    const DIGITS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let negative = num < 0.0;
    let value = num.abs();
    let int_part = value.trunc();
    let mut frac = value - int_part;

    // Integer part: collect base-`radix` digits least-significant first, then reverse.
    let mut int_digits = Vec::new();
    let mut i = int_part;
    if i == 0.0 {
        int_digits.push(b'0');
    }
    while i >= 1.0 {
        let d = (i % radix as f64) as usize;
        int_digits.push(DIGITS[d]);
        i = (i / radix as f64).trunc();
    }
    int_digits.reverse();
    let mut out = String::with_capacity(int_digits.len() + 2);
    if negative {
        out.push('-');
    }
    out.push_str(std::str::from_utf8(&int_digits).unwrap());

    // Fractional part: multiply-by-radix, emitting the integer overflow each step.
    if frac > 0.0 {
        out.push('.');
        let mut count = 0;
        while frac > 0.0 && count < 52 {
            frac *= radix as f64;
            let d = frac.trunc() as usize;
            out.push(DIGITS[d] as char);
            frac -= frac.trunc();
            count += 1;
        }
    }
    Some(out)
}
