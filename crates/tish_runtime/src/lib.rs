//! Minimal runtime for Tish compiled output.
//!
//! Provides Value representation, print, and heap/collection support
//! for native-compiled Tish programs.

use std::collections::HashMap;
use std::fmt;

/// Error type for Tish throw/catch.
#[derive(Debug, Clone)]
pub enum TishError {
    Throw(Value),
}

impl fmt::Display for TishError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TishError::Throw(v) => write!(f, "{}", v.to_display_string()),
        }
    }
}

impl std::error::Error for TishError {}
use std::rc::Rc;
use std::sync::Arc;

/// Native function type for first-class functions in compiled code.
pub type NativeFn = Rc<dyn Fn(&[Value]) -> Value>;

/// Runtime value used by compiled Tish programs.
#[derive(Clone)]
pub enum Value {
    Number(f64),
    String(Arc<str>),
    Bool(bool),
    Null,
    Array(Rc<Vec<Value>>),
    Object(Rc<HashMap<Arc<str>, Value>>),
    Function(NativeFn),
}

impl std::fmt::Debug for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Number(n) => write!(f, "Number({})", n),
            Value::String(s) => write!(f, "String({:?})", s.as_ref()),
            Value::Bool(b) => write!(f, "Bool({})", b),
            Value::Null => write!(f, "Null"),
            Value::Array(arr) => write!(f, "Array({:?})", arr.as_ref()),
            Value::Object(obj) => write!(f, "Object({:?})", obj.as_ref()),
            Value::Function(_) => write!(f, "Function"),
        }
    }
}

impl Value {
    pub fn to_display_string(&self) -> String {
        match self {
            Value::Number(n) => n.to_string(),
            Value::String(s) => s.to_string(),
            Value::Bool(b) => b.to_string(),
            Value::Null => "null".to_string(),
            Value::Array(arr) => {
                let inner: Vec<String> = arr.iter().map(|v| v.to_display_string()).collect();
                format!("[{}]", inner.join(", "))
            }
            Value::Object(obj) => {
                let inner: Vec<String> = obj
                    .iter()
                    .map(|(k, v)| format!("{}: {}", k.as_ref(), v.to_display_string()))
                    .collect();
                format!("{{{}}}", inner.join(", "))
            }
            Value::Function(_) => "[Function]".to_string(),
        }
    }

    pub fn is_truthy(&self) -> bool {
        match self {
            Value::Null => false,
            Value::Bool(b) => *b,
            Value::Number(n) => *n != 0.0 && !n.is_nan(),
            Value::String(s) => !s.is_empty(),
            _ => true,
        }
    }

    pub fn strict_eq(&self, other: &Value) -> bool {
        match (self, other) {
            (Value::Number(a), Value::Number(b)) => a == b,
            (Value::String(a), Value::String(b)) => a == b,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Null, Value::Null) => true,
            (Value::Array(a), Value::Array(b)) => Rc::ptr_eq(a, b),
            (Value::Object(a), Value::Object(b)) => Rc::ptr_eq(a, b),
            _ => false,
        }
    }
}

/// Builtin print: prints all arguments space-separated, then newline.
pub fn print(args: &[Value]) {
    let parts: Vec<String> = args.iter().map(Value::to_display_string).collect();
    println!("{}", parts.join(" "));
}

/// Builtin parseInt: parse string to integer. Optional radix 2-36.
pub fn parse_int(args: &[Value]) -> Value {
    let s = args
        .get(0)
        .map(Value::to_display_string)
        .unwrap_or_default();
    let s = s.trim();
    let radix = args
        .get(1)
        .and_then(|v| match v {
            Value::Number(n) => Some(*n as i32),
            _ => None,
        })
        .unwrap_or(10);
    if radix >= 2 && radix <= 36 {
        let prefix: String = s
            .chars()
            .take_while(|c| *c == '-' || *c == '+' || c.is_digit(radix as u32))
            .collect();
        if let Ok(n) = i64::from_str_radix(&prefix, radix as u32) {
            return Value::Number(n as f64);
        }
    }
    Value::Number(f64::NAN)
}

/// Builtin decodeURI: decode percent-encoded URI string.
pub fn decode_uri(args: &[Value]) -> Value {
    let s = args
        .get(0)
        .map(Value::to_display_string)
        .unwrap_or_default();
    let out = percent_decode(&s);
    Value::String(out.into())
}

/// Builtin encodeURI: encode URI string (preserves A-Za-z0-9;-/?:@&=+$,_.!~*'()).
pub fn encode_uri(args: &[Value]) -> Value {
    let s = args
        .get(0)
        .map(Value::to_display_string)
        .unwrap_or_default();
    let out = percent_encode(&s);
    Value::String(out.into())
}

fn percent_decode(s: &str) -> String {
    let mut out = Vec::new();
    let mut i = 0;
    let bytes = s.as_bytes();
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let h = (bytes[i + 1] as char).to_digit(16);
            let l = (bytes[i + 2] as char).to_digit(16);
            if let (Some(h), Some(l)) = (h, l) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn percent_encode(s: &str) -> String {
    const SAFE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789;-/?:@&=+$,_.!~*'()";
    let mut out = String::new();
    for c in s.chars() {
        if c.len_utf8() == 1 {
            let b = c as u32 as u8;
            if SAFE.contains(&b) {
                out.push(c);
                continue;
            }
        }
        for b in c.to_string().as_bytes() {
            out.push_str(&format!("%{:02X}", b));
        }
    }
    out
}

/// Builtin parseFloat: parse string to float.
pub fn parse_float(args: &[Value]) -> Value {
    let s = args
        .get(0)
        .map(Value::to_display_string)
        .unwrap_or_default();
    let n: f64 = s.trim().parse().unwrap_or(f64::NAN);
    Value::Number(n)
}

/// Builtin isFinite: true if value is finite number.
pub fn is_finite(args: &[Value]) -> Value {
    let b = args.get(0).map_or(false, |v| match v {
        Value::Number(n) => n.is_finite(),
        _ => false,
    });
    Value::Bool(b)
}

/// Builtin isNaN: true if value is NaN or not a number.
pub fn is_nan(args: &[Value]) -> Value {
    let b = args.get(0).map_or(true, |v| match v {
        Value::Number(n) => n.is_nan(),
        _ => true,
    });
    Value::Bool(b)
}

/// Math.abs
pub fn math_abs(args: &[Value]) -> Value {
    let n = args.get(0).and_then(|v| match v {
        Value::Number(n) => Some(*n),
        _ => None,
    }).unwrap_or(f64::NAN);
    Value::Number(n.abs())
}

/// Math.sqrt
pub fn math_sqrt(args: &[Value]) -> Value {
    let n = args.get(0).and_then(|v| match v {
        Value::Number(n) => Some(*n),
        _ => None,
    }).unwrap_or(f64::NAN);
    Value::Number(n.sqrt())
}

/// Math.min
pub fn math_min(args: &[Value]) -> Value {
    let nums: Vec<f64> = args
        .iter()
        .filter_map(|v| match v {
            Value::Number(n) => Some(*n),
            _ => None,
        })
        .collect();
    let n = nums.into_iter().fold(f64::INFINITY, f64::min);
    Value::Number(if n == f64::INFINITY { f64::NAN } else { n })
}

/// Math.max
pub fn math_max(args: &[Value]) -> Value {
    let nums: Vec<f64> = args
        .iter()
        .filter_map(|v| match v {
            Value::Number(n) => Some(*n),
            _ => None,
        })
        .collect();
    let n = nums.into_iter().fold(f64::NEG_INFINITY, f64::max);
    Value::Number(if n == f64::NEG_INFINITY { f64::NAN } else { n })
}

/// Math.floor
pub fn math_floor(args: &[Value]) -> Value {
    let n = args.get(0).and_then(|v| match v {
        Value::Number(n) => Some(*n),
        _ => None,
    }).unwrap_or(f64::NAN);
    Value::Number(n.floor())
}

/// Math.ceil
pub fn math_ceil(args: &[Value]) -> Value {
    let n = args.get(0).and_then(|v| match v {
        Value::Number(n) => Some(*n),
        _ => None,
    }).unwrap_or(f64::NAN);
    Value::Number(n.ceil())
}

/// Math.round
pub fn math_round(args: &[Value]) -> Value {
    let n = args.get(0).and_then(|v| match v {
        Value::Number(n) => Some(*n),
        _ => None,
    }).unwrap_or(f64::NAN);
    Value::Number(n.round())
}

/// Builtin 'in' operator: key in obj -> true if obj has property key.
pub fn in_operator(args: &[Value]) -> Value {
    let key_val = args.get(0);
    let obj = args.get(1);
    let (key, obj) = match (key_val, obj) {
        (Some(k), Some(o)) => (k, o),
        _ => return Value::Bool(false),
    };
    let key: Arc<str> = match key {
        Value::String(s) => Arc::clone(s),
        Value::Number(n) => n.to_string().into(),
        _ => return Value::Bool(false),
    };
    let ok = match obj {
        Value::Object(map) => map.contains_key(&key),
        Value::Array(arr) => {
            key.as_ref() == "length"
                || key
                    .parse::<usize>()
                    .ok()
                    .map(|i| i < arr.len())
                    .unwrap_or(false)
        }
        _ => false,
    };
    Value::Bool(ok)
}

/// Get property from object by string key.
pub fn get_prop(obj: &Value, key: impl AsRef<str>) -> Value {
    let key = key.as_ref();
    match obj {
        Value::Object(map) => {
            let k: Arc<str> = key.into();
            map.get(&k)
                .cloned()
                .unwrap_or(Value::Null)
        }
        Value::Array(arr) => {
            if key == "length" {
                Value::Number(arr.len() as f64)
            } else if let Ok(idx) = key.parse::<usize>() {
                arr.get(idx).cloned().unwrap_or(Value::Null)
            } else {
                Value::Null
            }
        }
        Value::String(s) => {
            if key == "length" {
                Value::Number(s.chars().count() as f64)
            } else {
                Value::Null
            }
        }
        _ => Value::Null,
    }
}

/// Get index from array or object.
pub fn get_index(obj: &Value, index: &Value) -> Value {
    match obj {
        Value::Array(arr) => {
            let idx = match index {
                Value::Number(n) => *n as usize,
                _ => return Value::Null,
            };
            arr.get(idx).cloned().unwrap_or(Value::Null)
        }
        Value::Object(map) => {
            let key: Arc<str> = match index {
                Value::Number(n) => n.to_string().into(),
                Value::String(s) => Arc::clone(s),
                _ => return Value::Null,
            };
            map.get(&key).cloned().unwrap_or(Value::Null)
        }
        _ => Value::Null,
    }
}

/// JSON.stringify - convert Value to JSON string
pub fn json_stringify(args: &[Value]) -> Value {
    let v = args.get(0).cloned().unwrap_or(Value::Null);
    Value::String(json_stringify_value(&v).into())
}

fn json_stringify_value(v: &Value) -> String {
    match v {
        Value::Null => "null".to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => {
            if n.is_finite() {
                n.to_string()
            } else if *n == f64::INFINITY {
                "null".to_string()
            } else if *n == f64::NEG_INFINITY {
                "null".to_string()
            } else {
                "null".to_string()
            }
        }
        Value::String(s) => format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n").replace('\r', "\\r").replace('\t', "\\t")),
        Value::Array(arr) => {
            let inner: Vec<String> = arr.iter().map(json_stringify_value).collect();
            format!("[{}]", inner.join(","))
        }
        Value::Object(map) => {
            let mut entries: Vec<_> = map
                .iter()
                .map(|(k, v)| {
                    (
                        k.as_ref(),
                        format!(
                            "\"{}\":{}",
                            k.replace('\\', "\\\\").replace('"', "\\\""),
                            json_stringify_value(v)
                        ),
                    )
                })
                .collect();
            entries.sort_by(|a, b| a.0.cmp(b.0));
            format!("{{{}}}", entries.into_iter().map(|(_, s)| s).collect::<Vec<_>>().join(","))
        }
        Value::Function(_) => "null".to_string(),
    }
}

/// JSON.parse - parse JSON string to Value
pub fn json_parse(args: &[Value]) -> Value {
    let s = args.get(0).map(|v| v.to_display_string()).unwrap_or_default();
    match json_parse_str(s.trim()) {
        Ok(v) => v,
        Err(_) => Value::Null,
    }
}

fn json_parse_str(s: &str) -> Result<Value, ()> {
    let s = s.trim();
    if s.is_empty() {
        return Err(());
    }
    if s == "null" {
        return Ok(Value::Null);
    }
    if s == "true" {
        return Ok(Value::Bool(true));
    }
    if s == "false" {
        return Ok(Value::Bool(false));
    }
    if s.starts_with('"') {
        return json_parse_string_full(s);
    }
    if s.starts_with('[') {
        return json_parse_array(s);
    }
    if s.starts_with('{') {
        return json_parse_object(s);
    }
    if let Ok(n) = s.parse::<f64>() {
        return Ok(Value::Number(n));
    }
    Err(())
}

fn json_parse_string(s: &str) -> Result<(Value, &str), ()> {
    let s = &s[1..];
    let mut out = String::new();
    let mut i = 0;
    let chars: Vec<char> = s.chars().collect();
    while i < chars.len() {
        if chars[i] == '"' {
            let rest_start = s.chars().take(i + 1).map(|c| c.len_utf8()).sum::<usize>();
            return Ok((Value::String(out.into()), &s[rest_start..]));
        }
        if chars[i] == '\\' {
            i += 1;
            if i >= chars.len() {
                return Err(());
            }
            match chars[i] {
                '"' => out.push('"'),
                '\\' => out.push('\\'),
                'n' => out.push('\n'),
                'r' => out.push('\r'),
                't' => out.push('\t'),
                _ => return Err(()),
            }
        } else {
            out.push(chars[i]);
        }
        i += 1;
    }
    Err(())
}

fn json_parse_string_full(s: &str) -> Result<Value, ()> {
    json_parse_string(s).map(|(v, _)| v)
}

fn json_parse_array(s: &str) -> Result<Value, ()> {
    let s = s[1..].trim_start();
    if s.starts_with(']') {
        return Ok(Value::Array(Rc::new(vec![])));
    }
    let mut vals = Vec::new();
    let mut rest = s;
    loop {
        let (v, next) = json_parse_one(rest)?;
        vals.push(v);
        rest = next.trim_start();
        if rest.starts_with(']') {
            break;
        }
        if !rest.starts_with(',') {
            return Err(());
        }
        rest = rest[1..].trim_start();
    }
    Ok(Value::Array(Rc::new(vals)))
}

fn json_parse_object(s: &str) -> Result<Value, ()> {
    let s = s[1..].trim_start();
    if s.starts_with('}') {
        return Ok(Value::Object(Rc::new(HashMap::new())));
    }
    let mut map = HashMap::new();
    let mut rest = s;
    loop {
        if !rest.starts_with('"') {
            return Err(());
        }
        let (key_val, next) = json_parse_string(rest)?;
        let key = match &key_val {
            Value::String(k) => Arc::clone(k),
            _ => return Err(()),
        };
        rest = next.trim_start();
        if !rest.starts_with(':') {
            return Err(());
        }
        rest = rest[1..].trim_start();
        let (val, next) = json_parse_one(rest)?;
        map.insert(key, val);
        rest = next.trim_start();
        if rest.starts_with('}') {
            break;
        }
        if !rest.starts_with(',') {
            return Err(());
        }
        rest = rest[1..].trim_start();
    }
    Ok(Value::Object(Rc::new(map)))
}

fn json_parse_one(s: &str) -> Result<(Value, &str), ()> {
    let s = s.trim();
    if s.is_empty() {
        return Err(());
    }
    if s.starts_with('"') {
        let (v, rest) = json_parse_string(s)?;
        Ok((v, rest))
    } else if s.starts_with('[') {
        let mut depth = 0;
        let mut i = 0;
        for c in s.chars() {
            if c == '[' {
                depth += 1;
            } else if c == ']' {
                depth -= 1;
                if depth == 0 {
                    let v = json_parse_array(&s[..=i])?;
                    return Ok((v, &s[i + 1..]));
                }
            }
            i += 1;
        }
        Err(())
    } else if s.starts_with('{') {
        let mut depth = 0;
        let mut i = 0;
        for c in s.chars() {
            if c == '{' {
                depth += 1;
            } else if c == '}' {
                depth -= 1;
                if depth == 0 {
                    let v = json_parse_object(&s[..=i])?;
                    return Ok((v, &s[i + 1..]));
                }
            }
            i += 1;
        }
        Err(())
    } else if s.starts_with("null") {
        Ok((Value::Null, &s[4..]))
    } else if s.starts_with("true") {
        Ok((Value::Bool(true), &s[4..]))
    } else if s.starts_with("false") {
        Ok((Value::Bool(false), &s[5..]))
    } else {
        let end = s
            .find(|c: char| !c.is_ascii_digit() && c != '-' && c != '+' && c != '.' && c != 'e' && c != 'E')
            .unwrap_or(s.len());
        let num_str = &s[..end];
        let n: f64 = num_str.parse().map_err(|_| ())?;
        Ok((Value::Number(n), &s[end..]))
    }
}
