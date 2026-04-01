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
    let s = s.trim();
    let radix = args.get(1).and_then(|v| match v {
        Value::Number(n) => Some(*n as i32),
        _ => None,
    }).unwrap_or(10);
    let n = if (2..=36).contains(&radix) {
        let prefix: String = s
            .chars()
            .take_while(|c| *c == '-' || *c == '+' || c.is_digit(radix as u32))
            .collect();
        i64::from_str_radix(&prefix, radix as u32).ok().map(|n| n as f64)
    } else {
        None
    };
    Ok(Value::Number(n.unwrap_or(f64::NAN)))
}

pub fn parse_float(args: &[Value]) -> Result<Value, String> {
    let s = args.first().map(|v| v.to_string()).unwrap_or_default();
    let n: f64 = s.trim().parse().unwrap_or(f64::NAN);
    Ok(Value::Number(n))
}

pub fn is_finite(args: &[Value]) -> Result<Value, String> {
    let b = args.first().is_some_and(|v| matches!(v, Value::Number(n) if n.is_finite()));
    Ok(Value::Bool(b))
}

pub fn is_nan(args: &[Value]) -> Result<Value, String> {
    let b = args.first().is_none_or(|v| matches!(v, Value::Number(n) if n.is_nan()) || !matches!(v, Value::Number(_)));
    Ok(Value::Bool(b))
}

pub fn boolean_native(args: &[Value]) -> Result<Value, String> {
    let v = args.first().unwrap_or(&Value::Null);
    Ok(Value::Bool(v.is_truthy()))
}

pub fn decode_uri(args: &[Value]) -> Result<Value, String> {
    let s = args.first().map(|v| v.to_string()).unwrap_or_default();
    Ok(Value::String(tishlang_core::percent_decode(&s).unwrap_or(s).into()))
}

pub fn encode_uri(args: &[Value]) -> Result<Value, String> {
    let s = args.first().map(|v| v.to_string()).unwrap_or_default();
    Ok(Value::String(tishlang_core::percent_encode(&s).into()))
}

pub fn math_abs(args: &[Value]) -> Result<Value, String> {
    Ok(Value::Number(get_num(args.first().unwrap_or(&Value::Null)).abs()))
}

pub fn math_sqrt(args: &[Value]) -> Result<Value, String> {
    Ok(Value::Number(get_num(args.first().unwrap_or(&Value::Null)).sqrt()))
}

pub fn math_min(args: &[Value]) -> Result<Value, String> {
    let nums: Vec<f64> = args.iter().filter_map(|v| match v {
        Value::Number(n) => Some(*n),
        _ => None,
    }).collect();
    let n = nums.into_iter().fold(f64::INFINITY, f64::min);
    Ok(Value::Number(if n == f64::INFINITY { f64::NAN } else { n }))
}

pub fn math_max(args: &[Value]) -> Result<Value, String> {
    let nums: Vec<f64> = args.iter().filter_map(|v| match v {
        Value::Number(n) => Some(*n),
        _ => None,
    }).collect();
    let n = nums.into_iter().fold(f64::NEG_INFINITY, f64::max);
    Ok(Value::Number(if n == f64::NEG_INFINITY { f64::NAN } else { n }))
}

pub fn math_floor(args: &[Value]) -> Result<Value, String> {
    Ok(Value::Number(get_num(args.first().unwrap_or(&Value::Null)).floor()))
}

pub fn math_ceil(args: &[Value]) -> Result<Value, String> {
    Ok(Value::Number(get_num(args.first().unwrap_or(&Value::Null)).ceil()))
}

pub fn math_round(args: &[Value]) -> Result<Value, String> {
    Ok(Value::Number(get_num(args.first().unwrap_or(&Value::Null)).round()))
}

pub fn math_random(_args: &[Value]) -> Result<Value, String> {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    let random = RandomState::new().build_hasher().finish() as f64 / u64::MAX as f64;
    Ok(Value::Number(random))
}

pub fn math_pow(args: &[Value]) -> Result<Value, String> {
    let base = get_num(args.first().unwrap_or(&Value::Null));
    let exp = get_num(args.get(1).unwrap_or(&Value::Null));
    Ok(Value::Number(base.powf(exp)))
}

pub fn math_sin(args: &[Value]) -> Result<Value, String> {
    Ok(Value::Number(get_num(args.first().unwrap_or(&Value::Null)).sin()))
}

pub fn math_cos(args: &[Value]) -> Result<Value, String> {
    Ok(Value::Number(get_num(args.first().unwrap_or(&Value::Null)).cos()))
}

pub fn math_tan(args: &[Value]) -> Result<Value, String> {
    Ok(Value::Number(get_num(args.first().unwrap_or(&Value::Null)).tan()))
}

pub fn math_log(args: &[Value]) -> Result<Value, String> {
    Ok(Value::Number(get_num(args.first().unwrap_or(&Value::Null)).ln()))
}

pub fn math_exp(args: &[Value]) -> Result<Value, String> {
    Ok(Value::Number(get_num(args.first().unwrap_or(&Value::Null)).exp()))
}

pub fn math_sign(args: &[Value]) -> Result<Value, String> {
    let n = get_num(args.first().unwrap_or(&Value::Null));
    let sign = if n.is_nan() { f64::NAN } else if n > 0.0 { 1.0 } else if n < 0.0 { -1.0 } else { 0.0 };
    Ok(Value::Number(sign))
}

pub fn math_trunc(args: &[Value]) -> Result<Value, String> {
    Ok(Value::Number(get_num(args.first().unwrap_or(&Value::Null)).trunc()))
}

pub fn date_now(_args: &[Value]) -> Result<Value, String> {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as f64)
        .unwrap_or(0.0);
    Ok(Value::Number(ms))
}

pub fn array_is_array(args: &[Value]) -> Result<Value, String> {
    Ok(Value::Bool(matches!(args.first(), Some(Value::Array(_)))))
}

pub fn string_from_char_code(args: &[Value]) -> Result<Value, String> {
    let s: String = args.iter().filter_map(|v| match v {
        Value::Number(n) => Some(char::from_u32(*n as u32).unwrap_or('\u{FFFD}')),
        _ => None,
    }).collect();
    Ok(Value::String(s.into()))
}

#[cfg(feature = "process")]
pub fn process_exit(args: &[Value]) -> Result<Value, String> {
    let code = args.first().and_then(|v| match v {
        Value::Number(n) => Some(*n as i32),
        _ => None,
    }).unwrap_or(0);
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

#[cfg(feature = "fs")]
pub fn read_file(args: &[Value]) -> Result<Value, String> {
    let path = args.first().map(|v| v.to_string()).unwrap_or_default();
    match std::fs::read_to_string(&path) {
        Ok(content) => Ok(Value::String(content.into())),
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

// ── tish:metal ──────────────────────────────────────────────────────────────

#[cfg(feature = "metal")]
fn make_result_obj(ms: f64, check: f64) -> Value {
    use crate::value::PropMap;
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::Arc;
    let mut map = PropMap::default();
    map.insert(Arc::from("ms"),    Value::Number(ms));
    map.insert(Arc::from("check"), Value::Number(check));
    Value::Object(Rc::new(RefCell::new(map)))
}

#[cfg(feature = "metal")]
pub fn metal_matmul_f32(args: &[Value]) -> Result<Value, String> {
    let n = match args.first() {
        Some(Value::Number(n)) => *n as usize,
        _ => return Err("metal_matmul_f32: expected number".into()),
    };
    let (ms, check) = tish_metal::matmul_f32(n)?;
    Ok(make_result_obj(ms, check))
}

#[cfg(feature = "metal")]
pub fn metal_device_name(_args: &[Value]) -> Result<Value, String> {
    Ok(Value::String(
        tish_metal::device_name()
            .unwrap_or_else(|| "no Metal device".into())
            .into(),
    ))
}

// ── tish:mlx ────────────────────────────────────────────────────────────────

#[cfg(feature = "mlx")]
fn make_mlx_result_obj(ms: f64, check: f64) -> Value {
    use crate::value::PropMap;
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::Arc;
    let mut map = PropMap::default();
    map.insert(Arc::from("ms"),    Value::Number(ms));
    map.insert(Arc::from("check"), Value::Number(check));
    Value::Object(Rc::new(RefCell::new(map)))
}

#[cfg(feature = "mlx")]
pub fn mlx_matmul_f32(args: &[Value]) -> Result<Value, String> {
    let n = match args.first() {
        Some(Value::Number(n)) => *n as usize,
        _ => return Err("mlx_matmul_f32: expected number".into()),
    };
    let (ms, check) = tish_mlx::matmul_f32(n)?;
    Ok(make_mlx_result_obj(ms, check))
}

#[cfg(feature = "mlx")]
pub fn mlx_device_name(_args: &[Value]) -> Result<Value, String> {
    Ok(Value::String(tish_mlx::device_name().into()))
}

#[cfg(feature = "mlx")]
pub fn mlx_version(_args: &[Value]) -> Result<Value, String> {
    Ok(Value::String(tish_mlx::version().into()))
}
