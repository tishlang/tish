//! Native function implementations for the interpreter.

use crate::value::Value;

fn get_num(v: &Value) -> f64 {
    match v {
        Value::Number(n) => *n,
        _ => f64::NAN,
    }
}

pub fn console_debug(args: &[Value]) -> Result<Value, String> {
    if get_log_level() == 0 {
        let parts: Vec<String> = args.iter().map(|v| v.to_string()).collect();
        println!("{}", parts.join(" "));
    }
    Ok(Value::Null)
}

pub fn console_info(args: &[Value]) -> Result<Value, String> {
    if get_log_level() <= 1 {
        let parts: Vec<String> = args.iter().map(|v| v.to_string()).collect();
        println!("{}", parts.join(" "));
    }
    Ok(Value::Null)
}

pub fn console_log(args: &[Value]) -> Result<Value, String> {
    if get_log_level() <= 2 {
        let parts: Vec<String> = args.iter().map(|v| v.to_string()).collect();
        println!("{}", parts.join(" "));
    }
    Ok(Value::Null)
}

pub fn console_warn(args: &[Value]) -> Result<Value, String> {
    if get_log_level() <= 3 {
        let parts: Vec<String> = args.iter().map(|v| v.to_string()).collect();
        eprintln!("{}", parts.join(" "));
    }
    Ok(Value::Null)
}

pub fn console_error(args: &[Value]) -> Result<Value, String> {
    let parts: Vec<String> = args.iter().map(|v| v.to_string()).collect();
    eprintln!("{}", parts.join(" "));
    Ok(Value::Null)
}

fn get_log_level() -> u8 {
    std::env::var("TISH_LOG_LEVEL")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(2)
}

pub fn parse_int(args: &[Value]) -> Result<Value, String> {
    let s = args.first().map(|v| v.to_string()).unwrap_or_default();
    let radix = args.get(1).and_then(|v| match v {
        Value::Number(n) => Some(*n as i32),
        _ => None,
    });
    // Shared semantics (0x stripping, sign, auto-radix) — see js_parse_int (#247).
    Ok(Value::Number(tishlang_builtins::globals::js_parse_int(
        &s, radix,
    )))
}

pub fn parse_float(args: &[Value]) -> Result<Value, String> {
    let s = args.first().map(|v| v.to_string()).unwrap_or_default();
    // JS parseFloat parses the longest leading numeric prefix (issue #36); shared with VM/native.
    Ok(Value::Number(tishlang_builtins::globals::js_parse_float(&s)))
}

pub fn is_finite(args: &[Value]) -> Result<Value, String> {
    // Global isFinite coerces via ToNumber (like Number()); absent arg (undefined) → NaN → false.
    Ok(Value::Bool(args.first().map_or(f64::NAN, to_number).is_finite()))
}

pub fn is_nan(args: &[Value]) -> Result<Value, String> {
    // Global isNaN coerces via ToNumber; absent arg (undefined) → NaN → true.
    Ok(Value::Bool(args.first().map_or(f64::NAN, to_number).is_nan()))
}

pub fn boolean_native(args: &[Value]) -> Result<Value, String> {
    let v = args.first().unwrap_or(&Value::Null);
    Ok(Value::Bool(v.is_truthy()))
}

pub fn decode_uri(args: &[Value]) -> Result<Value, String> {
    let s = args.first().map(|v| v.to_string()).unwrap_or_default();
    Ok(Value::String(
        tishlang_core::percent_decode(&s).unwrap_or(s).into(),
    ))
}

pub fn encode_uri(args: &[Value]) -> Result<Value, String> {
    let s = args.first().map(|v| v.to_string()).unwrap_or_default();
    Ok(Value::String(tishlang_core::percent_encode(&s).into()))
}

pub fn encode_uri_component(args: &[Value]) -> Result<Value, String> {
    let s = args.first().map(|v| v.to_string()).unwrap_or_default();
    Ok(Value::String(
        tishlang_core::percent_encode_component(&s).into(),
    ))
}

pub fn decode_uri_component(args: &[Value]) -> Result<Value, String> {
    let s = args.first().map(|v| v.to_string()).unwrap_or_default();
    Ok(Value::String(
        tishlang_core::percent_decode_component(&s).unwrap_or(s).into(),
    ))
}

pub fn html_escape(args: &[Value]) -> Result<Value, String> {
    let input = match args.first() {
        Some(Value::String(s)) => s.to_string(),
        Some(v) => v.to_string(),
        None => return Ok(Value::Null),
    };
    let bytes = input.as_bytes();
    let mut extra = 0usize;
    for b in bytes {
        match b {
            b'&' => extra += 4,
            b'<' | b'>' => extra += 3,
            b'"' => extra += 5,
            b'\'' => extra += 4,
            _ => {}
        }
    }
    if extra == 0 {
        return Ok(Value::String(input.into()));
    }
    let mut out = String::with_capacity(input.len() + extra);
    let mut last = 0usize;
    for (i, b) in bytes.iter().enumerate() {
        let repl: Option<&'static str> = match b {
            b'&' => Some("&amp;"),
            b'<' => Some("&lt;"),
            b'>' => Some("&gt;"),
            b'"' => Some("&quot;"),
            b'\'' => Some("&#39;"),
            _ => None,
        };
        if let Some(r) = repl {
            out.push_str(&input[last..i]);
            out.push_str(r);
            last = i + 1;
        }
    }
    out.push_str(&input[last..]);
    Ok(Value::String(out.into()))
}

pub fn math_abs(args: &[Value]) -> Result<Value, String> {
    Ok(Value::Number(
        get_num(args.first().unwrap_or(&Value::Null)).abs(),
    ))
}

pub fn math_sqrt(args: &[Value]) -> Result<Value, String> {
    Ok(Value::Number(
        get_num(args.first().unwrap_or(&Value::Null)).sqrt(),
    ))
}

// The JS-specific min/max/round semantics live in the shared `tishlang_builtins::math` f64 helpers
// (#247) — the interpreter uses a different `Value` type, so it extracts its own args and calls those
// helpers, keeping interp/vm/native bit-identical without duplicating the rules.
pub fn math_min(args: &[Value]) -> Result<Value, String> {
    let nums: Vec<f64> = args.iter().map(get_num).collect();
    Ok(Value::Number(tishlang_builtins::math::min_f64(&nums)))
}

pub fn math_max(args: &[Value]) -> Result<Value, String> {
    let nums: Vec<f64> = args.iter().map(get_num).collect();
    Ok(Value::Number(tishlang_builtins::math::max_f64(&nums)))
}

// hypot/asin/acos/atan/atan2 existed on the vm's Math but not the interpreter's — a direct interp≠vm
// gap (#247). asin/acos/atan/atan2 are 1:1 with Rust f64 methods, so compute directly. hypot is
// VARIADIC in JS and must propagate NaN/Infinity, so it delegates to the shared `hypot_f64` (exactly
// like min/max) — a 2-arg `x.hypot(y)` gave `Math.hypot(3,4,12)` = 5 instead of 13.
pub fn math_hypot(args: &[Value]) -> Result<Value, String> {
    let nums: Vec<f64> = args.iter().map(get_num).collect();
    Ok(Value::Number(tishlang_builtins::math::hypot_f64(&nums)))
}

pub fn math_asin(args: &[Value]) -> Result<Value, String> {
    Ok(Value::Number(get_num(args.first().unwrap_or(&Value::Null)).asin()))
}

pub fn math_acos(args: &[Value]) -> Result<Value, String> {
    Ok(Value::Number(get_num(args.first().unwrap_or(&Value::Null)).acos()))
}

pub fn math_atan(args: &[Value]) -> Result<Value, String> {
    Ok(Value::Number(get_num(args.first().unwrap_or(&Value::Null)).atan()))
}

pub fn math_atan2(args: &[Value]) -> Result<Value, String> {
    let y = get_num(args.first().unwrap_or(&Value::Null));
    let x = get_num(args.get(1).unwrap_or(&Value::Null));
    Ok(Value::Number(y.atan2(x)))
}

pub fn math_floor(args: &[Value]) -> Result<Value, String> {
    Ok(Value::Number(
        get_num(args.first().unwrap_or(&Value::Null)).floor(),
    ))
}

pub fn math_ceil(args: &[Value]) -> Result<Value, String> {
    Ok(Value::Number(
        get_num(args.first().unwrap_or(&Value::Null)).ceil(),
    ))
}

pub fn math_round(args: &[Value]) -> Result<Value, String> {
    Ok(Value::Number(tishlang_builtins::math::round_f64(get_num(
        args.first().unwrap_or(&Value::Null),
    ))))
}

pub fn math_random(_args: &[Value]) -> Result<Value, String> {
    // Match the VM / builtins (`rand::random::<f64>()`) — uniform [0,1). The old
    // RandomState-hash was non-uniform and diverged from every other backend.
    Ok(Value::Number(rand::random::<f64>()))
}

pub fn math_pow(args: &[Value]) -> Result<Value, String> {
    let base = get_num(args.first().unwrap_or(&Value::Null));
    let exp = get_num(args.get(1).unwrap_or(&Value::Null));
    Ok(Value::Number(base.powf(exp)))
}

/// ES6 `Math.imul`: an exact 32-bit integer multiply. Both operands go through JS `ToInt32`
/// (modulo 2³², NaN/±Infinity → 0), NOT a saturating `as i32` cast, then a wrapping i32 multiply —
/// bit-for-bit identical to the shared `tishlang_builtins::math::imul`, the vm, the native path, and
/// V8. Was previously absent from the interpreter's `Math` object, so `Math.imul(...)` threw
/// "Not a function" in `--backend interp` while working natively (a cross-backend divergence).
pub fn math_imul(args: &[Value]) -> Result<Value, String> {
    let a = tishlang_core::to_int32(get_num(args.first().unwrap_or(&Value::Null)));
    let b = tishlang_core::to_int32(get_num(args.get(1).unwrap_or(&Value::Null)));
    Ok(Value::Number(a.wrapping_mul(b) as f64))
}

pub fn math_sin(args: &[Value]) -> Result<Value, String> {
    Ok(Value::Number(
        get_num(args.first().unwrap_or(&Value::Null)).sin(),
    ))
}

pub fn math_cos(args: &[Value]) -> Result<Value, String> {
    Ok(Value::Number(
        get_num(args.first().unwrap_or(&Value::Null)).cos(),
    ))
}

pub fn math_tan(args: &[Value]) -> Result<Value, String> {
    Ok(Value::Number(
        get_num(args.first().unwrap_or(&Value::Null)).tan(),
    ))
}

pub fn math_log(args: &[Value]) -> Result<Value, String> {
    Ok(Value::Number(
        get_num(args.first().unwrap_or(&Value::Null)).ln(),
    ))
}

pub fn math_exp(args: &[Value]) -> Result<Value, String> {
    Ok(Value::Number(
        get_num(args.first().unwrap_or(&Value::Null)).exp(),
    ))
}

pub fn math_sign(args: &[Value]) -> Result<Value, String> {
    let n = get_num(args.first().unwrap_or(&Value::Null));
    let sign = if n.is_nan() {
        f64::NAN
    } else if n > 0.0 {
        1.0
    } else if n < 0.0 {
        -1.0
    } else {
        // ±0 → return the input so `Math.sign(-0)` is `-0` (per ES); bare `0.0` drops the sign.
        n
    };
    Ok(Value::Number(sign))
}

pub fn math_trunc(args: &[Value]) -> Result<Value, String> {
    Ok(Value::Number(
        get_num(args.first().unwrap_or(&Value::Null)).trunc(),
    ))
}

// Hyperbolic / inverse-hyperbolic / cbrt / base-2/10 logs — these were missing from the
// interpreter's `Math`, so they returned "Not a function" (issue #61, and an interp↔VM
// divergence under #67). One macro per unary `f64` method keeps them in lockstep with the VM.
macro_rules! math_unary {
    ($name:ident, $method:ident) => {
        pub fn $name(args: &[Value]) -> Result<Value, String> {
            Ok(Value::Number(
                get_num(args.first().unwrap_or(&Value::Null)).$method(),
            ))
        }
    };
}
math_unary!(math_sinh, sinh);
math_unary!(math_cosh, cosh);
math_unary!(math_tanh, tanh);
math_unary!(math_asinh, asinh);
math_unary!(math_acosh, acosh);
math_unary!(math_atanh, atanh);
math_unary!(math_cbrt, cbrt);
math_unary!(math_log2, log2);
math_unary!(math_log10, log10);
math_unary!(math_expm1, exp_m1);
math_unary!(math_log1p, ln_1p);

pub fn math_clz32(args: &[Value]) -> Result<Value, String> {
    let n = tishlang_core::to_uint32(get_num(args.first().unwrap_or(&Value::Null)));
    Ok(Value::Number(n.leading_zeros() as f64))
}
pub fn math_fround(args: &[Value]) -> Result<Value, String> {
    let x = get_num(args.first().unwrap_or(&Value::Null));
    Ok(Value::Number(x as f32 as f64))
}

pub fn array_is_array(args: &[Value]) -> Result<Value, String> {
    Ok(Value::Bool(matches!(args.first(), Some(Value::Array(_)))))
}

/// Build a JS-style error object `{ name, message }` for the interpreter (issue #60).
fn make_error_obj(name: &str, args: &[Value]) -> Value {
    let message = args.first().map(|v| v.to_string()).unwrap_or_default();
    let mut m = crate::value::PropMap::with_capacity(2);
    m.insert("name".into(), Value::String(name.into()));
    m.insert("message".into(), Value::String(message.into()));
    Value::object(m)
}

pub fn error_construct(args: &[Value]) -> Result<Value, String> {
    Ok(make_error_obj("Error", args))
}
pub fn type_error_construct(args: &[Value]) -> Result<Value, String> {
    Ok(make_error_obj("TypeError", args))
}
pub fn range_error_construct(args: &[Value]) -> Result<Value, String> {
    Ok(make_error_obj("RangeError", args))
}
pub fn syntax_error_construct(args: &[Value]) -> Result<Value, String> {
    Ok(make_error_obj("SyntaxError", args))
}

/// `Array(...)` / `new Array(...)` (issue #72) — mirrors `tishlang_builtins::construct::
/// array_construct` for the interpreter's `Value`. A single non-negative integer is a length
/// (null holes); other args become the array's elements.
pub fn array_construct(args: &[Value]) -> Result<Value, String> {
    if let [Value::Number(n)] = args {
        let n = *n;
        if n >= 0.0 && n.fract() == 0.0 && n <= 4_294_967_295.0 {
            return Ok(Value::array(vec![Value::Null; n as usize]));
        }
    }
    Ok(Value::array(args.to_vec()))
}

/// `Array.of(...items)` — args become elements (a single number is NOT a length, unlike `Array(n)`).
pub fn array_of(args: &[Value]) -> Result<Value, String> {
    Ok(Value::array(args.to_vec()))
}

/// `Object.is(a, b)` — SameValue: like `===`, but NaN equals NaN and `+0` is not `-0`.
pub fn object_is(args: &[Value]) -> Result<Value, String> {
    let a = args.first().unwrap_or(&Value::Null);
    let b = args.get(1).unwrap_or(&Value::Null);
    let same = match (a, b) {
        (Value::Number(x), Value::Number(y)) => {
            if x.is_nan() && y.is_nan() {
                true
            } else if *x == 0.0 && *y == 0.0 {
                x.is_sign_negative() == y.is_sign_negative()
            } else {
                x == y
            }
        }
        _ => a.strict_eq(b),
    };
    Ok(Value::Bool(same))
}

/// `String(value)` as a function: JS `ToString` coercion (arrays comma-join recursively,
/// objects → "[object Object]", null → "null"), matching the VM/native `string_convert`.
pub fn string_convert(args: &[Value]) -> Result<Value, String> {
    let v = args.first().unwrap_or(&Value::Null);
    Ok(Value::String(v.to_js_string().into()))
}

/// `Number(value)` coercion (issue #36) — shares the string parser with the VM/native
/// backend so the result is byte-identical.
pub fn number_convert(args: &[Value]) -> Result<Value, String> {
    Ok(Value::Number(to_number(args.first().unwrap_or(&Value::Null))))
}

/// JS `ToNumber(v)` for the interpreter's Value — shares the string parser with the VM/native path
/// so `Number()`, `isNaN`, `isFinite` are byte-identical across backends.
pub fn to_number(v: &Value) -> f64 {
    match v {
        Value::Number(n) => *n,
        Value::Bool(b) => {
            if *b {
                1.0
            } else {
                0.0
            }
        }
        Value::Null => 0.0,
        Value::String(s) => tishlang_builtins::globals::parse_numeric_string(s),
        other => tishlang_builtins::globals::parse_numeric_string(&other.to_js_string()),
    }
}

// `Number.*` static predicates — no coercion (the coercing forms are the bare globals).
pub fn number_is_integer(args: &[Value]) -> Result<Value, String> {
    Ok(Value::Bool(matches!(args.first(), Some(Value::Number(n)) if n.is_finite() && n.fract() == 0.0)))
}
pub fn number_is_safe_integer(args: &[Value]) -> Result<Value, String> {
    Ok(Value::Bool(matches!(args.first(),
        Some(Value::Number(n)) if n.is_finite() && n.fract() == 0.0 && n.abs() <= 9_007_199_254_740_991.0)))
}
pub fn number_is_nan(args: &[Value]) -> Result<Value, String> {
    Ok(Value::Bool(matches!(args.first(), Some(Value::Number(n)) if n.is_nan())))
}
pub fn number_is_finite(args: &[Value]) -> Result<Value, String> {
    Ok(Value::Bool(matches!(args.first(), Some(Value::Number(n)) if n.is_finite())))
}

pub fn string_from_char_code(args: &[Value]) -> Result<Value, String> {
    let s: String = args
        .iter()
        .filter_map(|v| match v {
            Value::Number(n) => Some(char::from_u32(*n as u32).unwrap_or('\u{FFFD}')),
            _ => None,
        })
        .collect();
    Ok(Value::String(s.into()))
}

#[cfg(feature = "process")]
pub fn process_exit(args: &[Value]) -> Result<Value, String> {
    let code = args
        .first()
        .and_then(|v| match v {
            Value::Number(n) => Some(*n as i32),
            _ => None,
        })
        .unwrap_or(0);
    std::process::exit(code);
}

#[cfg(feature = "process")]
pub fn process_cwd(_args: &[Value]) -> Result<Value, String> {
    let cwd = std::env::current_dir()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    Ok(Value::String(cwd.into()))
}

#[cfg(feature = "process")]
pub fn process_exec(args: &[Value]) -> Result<Value, String> {
    use std::process::Command;
    let cmd = args.first().map(|v| v.to_string()).unwrap_or_default();
    if cmd.is_empty() {
        return Ok(Value::Number(0.0));
    }
    let output = Command::new("sh")
        .arg("-c")
        .arg(&cmd)
        .output()
        .map_err(|e| format!("exec failed: {}", e))?;
    let code = output.status.code().unwrap_or(1);
    Ok(Value::Number(code as f64))
}

/// `process.execFile(program, [args])` — run a program directly, without a shell, passing each arg
/// verbatim (the safe, no-`sh -c` counterpart to `exec`). Returns the exit code. #384
pub fn process_exec_file(args: &[Value]) -> Result<Value, String> {
    use std::process::Command;
    let program = args.first().map(|v| v.to_string()).unwrap_or_default();
    if program.is_empty() {
        return Ok(Value::Number(0.0));
    }
    let argv: Vec<String> = match args.get(1) {
        Some(Value::Array(a)) => a.borrow().iter().map(|v| v.to_string()).collect(),
        _ => Vec::new(),
    };
    let output = Command::new(&program)
        .args(&argv)
        .output()
        .map_err(|e| format!("execFile failed: {}", e))?;
    Ok(Value::Number(output.status.code().unwrap_or(1) as f64))
}

#[cfg(feature = "fs")]
pub fn read_file(args: &[Value]) -> Result<Value, String> {
    let path = args.first().map(|v| v.to_string()).unwrap_or_default();
    match std::fs::read_to_string(&path) {
        Ok(content) => Ok(Value::String(content.into())),
        Err(e) => Ok(Value::String(format!("Error: {}", e).into())),
    }
}

/// Read a file as raw bytes (array of numbers 0–255) for binary data that `read_file`
/// (UTF-8 only) can't handle. See issue #120.
#[cfg(feature = "fs")]
pub fn read_file_bytes(args: &[Value]) -> Result<Value, String> {
    use std::cell::RefCell;
    use std::rc::Rc;
    let path = args.first().map(|v| v.to_string()).unwrap_or_default();
    match std::fs::read(&path) {
        Ok(bytes) => {
            let items: Vec<Value> = bytes.into_iter().map(|b| Value::Number(b as f64)).collect();
            Ok(Value::Array(Rc::new(RefCell::new(items))))
        }
        Err(e) => Ok(Value::String(format!("Error: {}", e).into())),
    }
}

#[cfg(feature = "fs")]
pub fn write_file(args: &[Value]) -> Result<Value, String> {
    let path = args.first().map(|v| v.to_string()).unwrap_or_default();
    let content = args.get(1).map(|v| v.to_string()).unwrap_or_default();
    match std::fs::write(&path, content) {
        Ok(_) => Ok(Value::Bool(true)),
        Err(_) => Ok(Value::Bool(false)),
    }
}

#[cfg(feature = "fs")]
pub fn file_exists(args: &[Value]) -> Result<Value, String> {
    let path = args.first().map(|v| v.to_string()).unwrap_or_default();
    Ok(Value::Bool(std::path::Path::new(&path).exists()))
}

#[cfg(feature = "fs")]
pub fn is_dir(args: &[Value]) -> Result<Value, String> {
    let path = args.first().map(|v| v.to_string()).unwrap_or_default();
    Ok(Value::Bool(std::path::Path::new(&path).is_dir()))
}

#[cfg(feature = "fs")]
pub fn read_dir(args: &[Value]) -> Result<Value, String> {
    use std::cell::RefCell;
    use std::rc::Rc;

    let path = args.first().map(|v| v.to_string()).unwrap_or_default();
    match std::fs::read_dir(&path) {
        Ok(entries) => {
            let items: Vec<Value> = entries
                .filter_map(|e| e.ok())
                .map(|e| Value::String(e.file_name().to_string_lossy().into()))
                .collect();
            Ok(Value::Array(Rc::new(RefCell::new(items))))
        }
        Err(_) => Ok(Value::Array(Rc::new(RefCell::new(Vec::new())))),
    }
}

#[cfg(feature = "fs")]
pub fn mkdir(args: &[Value]) -> Result<Value, String> {
    let path = args.first().map(|v| v.to_string()).unwrap_or_default();
    let recursive = args.get(1).map(|v| v.is_truthy()).unwrap_or(false);
    let result = if recursive {
        std::fs::create_dir_all(&path)
    } else {
        std::fs::create_dir(&path)
    };
    Ok(Value::Bool(result.is_ok()))
}

// ── Interactive terminal I/O (issue #101), behind the `tty` feature ──────────────────────
// Build the interpreter's `Value` from the shared, Value-agnostic core in
// `tishlang_runtime::tty`, so the interpreter, VM, and native backends behave identically.

#[cfg(feature = "tty")]
fn tty_obj(pairs: Vec<(&str, Value)>) -> Value {
    let mut m = crate::value::PropMap::with_capacity(pairs.len());
    for (k, v) in pairs {
        m.insert(k.into(), v);
    }
    Value::object(m)
}

#[cfg(feature = "tty")]
pub fn tty_size(_args: &[Value]) -> Result<Value, String> {
    Ok(match tishlang_runtime::tty::size() {
        Some((cols, rows)) => tty_obj(vec![
            ("cols", Value::Number(cols as f64)),
            ("rows", Value::Number(rows as f64)),
        ]),
        None => Value::Null,
    })
}

#[cfg(feature = "tty")]
pub fn tty_is_tty(_args: &[Value]) -> Result<Value, String> {
    Ok(Value::Bool(tishlang_runtime::tty::is_tty()))
}

#[cfg(feature = "tty")]
pub fn tty_set_raw_mode(args: &[Value]) -> Result<Value, String> {
    let on = args.first().map(|v| v.is_truthy()).unwrap_or(false);
    Ok(Value::Bool(tishlang_runtime::tty::set_raw_mode(on)))
}

#[cfg(feature = "tty")]
pub fn tty_enter_alt_screen(_args: &[Value]) -> Result<Value, String> {
    Ok(Value::Bool(tishlang_runtime::tty::enter_alt_screen()))
}

#[cfg(feature = "tty")]
pub fn tty_leave_alt_screen(_args: &[Value]) -> Result<Value, String> {
    Ok(Value::Bool(tishlang_runtime::tty::leave_alt_screen()))
}

#[cfg(feature = "tty")]
pub fn tty_read_line(_args: &[Value]) -> Result<Value, String> {
    Ok(match tishlang_runtime::tty::read_line() {
        Some(s) => Value::String(s.into()),
        None => Value::Null,
    })
}

#[cfg(feature = "tty")]
pub fn tty_read(args: &[Value]) -> Result<Value, String> {
    use tishlang_runtime::tty::TtyEvent;
    let timeout = match args.first() {
        Some(Value::Number(ms)) => Some(ms.max(0.0) as u64),
        _ => None,
    };
    Ok(match tishlang_runtime::tty::read_event(timeout) {
        Some(TtyEvent::Key { key, ctrl, alt, shift }) => tty_obj(vec![
            ("type", Value::String("key".into())),
            ("key", Value::String(key.into())),
            ("ctrl", Value::Bool(ctrl)),
            ("alt", Value::Bool(alt)),
            ("shift", Value::Bool(shift)),
        ]),
        Some(TtyEvent::Resize { cols, rows }) => tty_obj(vec![
            ("type", Value::String("resize".into())),
            ("cols", Value::Number(cols as f64)),
            ("rows", Value::Number(rows as f64)),
        ]),
        Some(TtyEvent::Other) => tty_obj(vec![("type", Value::String("other".into()))]),
        None => Value::Null,
    })
}
