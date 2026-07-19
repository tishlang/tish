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
/// exact behavior. tish's rule (#438): round the number's *true* decimal value to `digits` places,
/// ties away from zero — matching V8/node's observable `toFixed`.
///
/// The obvious `(num * 10^digits).round()` is wrong: float scaling nudges values that are actually
/// just below a tie up over it, so `(2.675).toFixed(2)` came out `2.68` where node gives `2.67`
/// (2.675 is stored as 2.67499999…). Instead we render the double's exact decimal expansion to well
/// beyond `digits` — f64 fractions terminate, so a generous precision captures the real digits — then
/// round that digit string: if the first dropped digit is ≥ 5, increment with decimal carry; else
/// truncate. On the magnitude, "first dropped ≥ 5" is round-half-up = round-half-away-from-zero, and
/// because the expansion is exact, non-ties round the way the true value dictates. #438
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

    let negative = num.is_sign_negative() && num != 0.0;
    let magnitude = num.abs();

    // Exact-enough decimal expansion. The digit just past `digits` then reflects the double's TRUE
    // value; +25 guard digits sit far beyond any position we inspect, so the half-to-even rounding
    // Rust applies at the cutoff can't perturb our decision (it would take 25 consecutive 9s to carry
    // back that far, and such a value already rounds up under the exact rule anyway).
    let guard = digits + 25;
    let rendered = format!("{:.*}", guard, magnitude);
    let dot = rendered.find('.').expect("fixed formatting always has a decimal point");
    let int_str = &rendered[..dot];
    let frac = rendered[dot + 1..].as_bytes();

    // Kept digits: the whole integer part, then `digits` fractional digits.
    let mut ds: Vec<u8> = int_str
        .bytes()
        .chain(frac[..digits].iter().copied())
        .map(|b| b - b'0')
        .collect();

    // Round half-away-from-zero on the magnitude: increment iff the first dropped digit is >= 5.
    if frac[digits] - b'0' >= 5 {
        let mut i = ds.len();
        loop {
            if i == 0 {
                ds.insert(0, 1);
                break;
            }
            i -= 1;
            if ds[i] == 9 {
                ds[i] = 0;
            } else {
                ds[i] += 1;
                break;
            }
        }
    }

    // Split back into integer (all but the last `digits`) and fractional digits.
    let int_len = ds.len() - digits;
    let mut out = String::with_capacity(ds.len() + 2);
    if negative {
        out.push('-');
    }
    for &d in &ds[..int_len] {
        out.push((d + b'0') as char);
    }
    if digits > 0 {
        out.push('.');
        for &d in &ds[int_len..] {
            out.push((d + b'0') as char);
        }
    }
    out
}

/// `Number.prototype.toExponential(fractionDigits?)` — ECMA-262 §21.1.3.2. Exponential notation with
/// `fractionDigits` mantissa fraction digits (0–100), or the minimal digits needed when omitted. The
/// exponent always carries an explicit sign (`1.23e+4`, `1e-7`) to match V8.
pub fn to_exponential(n: &Value, digits: &Value) -> Value {
    let num = match n {
        Value::Number(x) => *x,
        _ => f64::NAN,
    };
    let d = match digits {
        Value::Number(x) => Some((*x as i32).clamp(0, 100) as usize),
        _ => None,
    };
    Value::String(to_exponential_str(num, d).into())
}

/// f64-domain core of `toExponential`, shared with the tree-walk interpreter.
pub fn to_exponential_str(num: f64, digits: Option<usize>) -> String {
    if num.is_nan() {
        return "NaN".to_string();
    }
    if num.is_infinite() {
        return if num < 0.0 { "-Infinity" } else { "Infinity" }.to_string();
    }
    let d = match digits {
        // No fractionDigits: shortest round-trip mantissa (Rust's `{:e}` matches V8 here).
        None => return fix_exponent_sign(&format!("{:e}", num)),
        Some(d) => d,
    };
    if num == 0.0 {
        let mant = if d == 0 {
            "0".to_string()
        } else {
            format!("0.{}", "0".repeat(d))
        };
        return format!("{}e+0", mant);
    }
    let sign = if num < 0.0 { "-" } else { "" };
    let (ds, e) = sig_digits(num.abs(), d + 1);
    let mant = if d == 0 {
        ds
    } else {
        format!("{}.{}", &ds[..1], &ds[1..])
    };
    let esign = if e >= 0 { "+" } else { "-" };
    format!("{}{}e{}{}", sign, mant, esign, e.abs())
}

/// Round `x` (finite, `x > 0`) to `sig` significant decimal digits with **half-away-from-zero**
/// rounding — ECMA's "pick the larger n" tie rule (`2.5.toPrecision(1) === "3"`, not "2"). Returns the
/// `sig`-digit string and the base-10 exponent of the leading digit.
///
/// The exact value of an f64 has a finite decimal expansion; scaling it through `f64` arithmetic
/// re-rounds and can land a genuinely-below-half value (e.g. `2.675` ≈ `2.67499…`) exactly on `.5`,
/// corrupting the tie test. So we format with many guard digits — `{:.Ne}` gives the correctly-rounded
/// exact-value expansion — capturing the TRUE digit at position `sig`, then round the digit string:
/// `digit[sig] >= 5` rounds up (half-away; `== 5` is a true tie only because the guard digits show no
/// nonzero remainder). Rust's `{:e}` alone rounds half-to-even, which is exactly what we must avoid.
fn sig_digits(x: f64, sig: usize) -> (String, i32) {
    let sig = sig.max(1);
    // Guard digits push the format's own (half-to-even) rounding far to the right of digit[sig], so
    // digit[sig] is exact. A cascade into digit[sig] would need 16 consecutive 9s — not a real input.
    let guard = sig + 16;
    let raw = format!("{:.*e}", guard, x);
    let (mant, exp_str) = raw.split_once('e').unwrap();
    let mut e: i32 = exp_str.parse().unwrap_or(0);
    let mut digits: Vec<u8> = mant
        .bytes()
        .filter(u8::is_ascii_digit)
        .map(|b| b - b'0')
        .collect();
    if digits.len() > sig {
        let round_up = digits[sig] >= 5;
        digits.truncate(sig);
        if round_up {
            let mut i = sig;
            loop {
                if i == 0 {
                    // carry out of the leading digit: 9…9 → 1 0…0, one more place
                    digits.insert(0, 1);
                    digits.truncate(sig);
                    e += 1;
                    break;
                }
                i -= 1;
                if digits[i] == 9 {
                    digits[i] = 0;
                } else {
                    digits[i] += 1;
                    break;
                }
            }
        }
    }
    let s: String = digits.iter().map(|d| (d + b'0') as char).collect();
    (s, e)
}

/// Rust's `{:e}` writes a bare positive exponent (`1.23e4`); JS/V8 always signs it (`1.23e+4`). Negative
/// exponents already carry `-`.
fn fix_exponent_sign(s: &str) -> String {
    match s.split_once('e') {
        Some((mantissa, exp)) if !exp.starts_with('-') && !exp.starts_with('+') => {
            format!("{}e+{}", mantissa, exp)
        }
        _ => s.to_string(),
    }
}

/// `Number.prototype.toPrecision(precision?)` — ECMA-262 §21.1.3.5. With `precision` significant digits,
/// choosing fixed or exponential per the spec; omitted precision falls back to `ToString`. An
/// out-of-range precision (`<1` or `>100`) parks a catchable `RangeError`.
pub fn to_precision(n: &Value, precision: &Value) -> Value {
    let num = match n {
        Value::Number(x) => *x,
        _ => f64::NAN,
    };
    match precision {
        Value::Number(p) => {
            let p = *p as i32;
            if !(1..=100).contains(&p) && num.is_finite() {
                tishlang_core::set_pending_throw(tishlang_core::range_error(
                    "toPrecision() argument must be between 1 and 100",
                ));
                return Value::Null;
            }
            Value::String(to_precision_str(num, p).into())
        }
        // precision undefined → behave like ToString(number)
        _ => Value::String(tishlang_core::js_number_to_string(num).into()),
    }
}

/// f64-domain core of `toPrecision`, shared with the tree-walk interpreter. Assumes `precision` is in
/// range (the `&Value` entry point validates and parks a RangeError otherwise).
pub fn to_precision_str(num: f64, precision: i32) -> String {
    if num.is_nan() {
        return "NaN".to_string();
    }
    if num.is_infinite() {
        return if num < 0.0 { "-Infinity" } else { "Infinity" }.to_string();
    }
    let p = precision.clamp(1, 100) as usize;
    if num == 0.0 {
        if p == 1 {
            return "0".to_string();
        }
        return format!("0.{}", "0".repeat(p - 1));
    }
    let sign = if num < 0.0 { "-" } else { "" };
    // `p` significant digits + base-10 exponent, half-away rounded (shared with toExponential).
    let (digits, e) = sig_digits(num.abs(), p);
    let body = if e < -6 || e >= p as i32 {
        // exponential form
        let mut m = String::new();
        m.push(digits.as_bytes()[0] as char);
        if p > 1 {
            m.push('.');
            m.push_str(&digits[1..]);
        }
        let esign = if e >= 0 { "+" } else { "-" };
        format!("{}e{}{}", m, esign, e.abs())
    } else if e == p as i32 - 1 {
        digits
    } else if e >= 0 {
        let split = (e + 1) as usize;
        format!("{}.{}", &digits[..split], &digits[split..])
    } else {
        let zeros = (-e - 1) as usize;
        format!("0.{}{}", "0".repeat(zeros), digits)
    };
    format!("{}{}", sign, body)
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
