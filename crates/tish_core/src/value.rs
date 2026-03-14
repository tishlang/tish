//! Unified Value type for Tish runtime values.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

#[cfg(feature = "regex")]
use fancy_regex::Regex;

/// Native function signature.
/// Returns Value directly (not Result) for simplicity and backward compatibility.
pub type NativeFn = Rc<dyn Fn(&[Value]) -> Value>;

/// Trait for opaque Rust types exposed to Tish (e.g. Polars DataFrame).
/// Implementors provide method dispatch so Tish can call methods on the value.
pub trait TishOpaque: Send + Sync {
    /// Display name for the type (e.g. "DataFrame").
    fn type_name(&self) -> &'static str;

    /// Get a method by name. Returns a native function if the method exists.
    fn get_method(&self, name: &str) -> Option<NativeFn>;
}

/// Trait for Promise-like values that can be awaited (block until settled).
/// Implemented by the runtime for native compile; interpreter uses its own Promise.
pub trait TishPromise: Send + Sync {
    fn block_until_settled(&self) -> std::result::Result<Value, Value>;
}

/// JavaScript RegExp flags
#[cfg(feature = "regex")]
#[derive(Debug, Clone, Default)]
pub struct RegExpFlags {
    pub global: bool,
    pub ignore_case: bool,
    pub multiline: bool,
    pub dot_all: bool,
    pub unicode: bool,
    pub sticky: bool,
}

#[cfg(feature = "regex")]
impl RegExpFlags {
    pub fn from_string(flags: &str) -> Result<Self, String> {
        let mut result = Self::default();
        for c in flags.chars() {
            match c {
                'g' => { if result.global { return Err(format!("duplicate flag '{}'", c)); } result.global = true; }
                'i' => { if result.ignore_case { return Err(format!("duplicate flag '{}'", c)); } result.ignore_case = true; }
                'm' => { if result.multiline { return Err(format!("duplicate flag '{}'", c)); } result.multiline = true; }
                's' => { if result.dot_all { return Err(format!("duplicate flag '{}'", c)); } result.dot_all = true; }
                'u' => { if result.unicode { return Err(format!("duplicate flag '{}'", c)); } result.unicode = true; }
                'y' => { if result.sticky { return Err(format!("duplicate flag '{}'", c)); } result.sticky = true; }
                _ => return Err(format!("unknown flag '{}'", c)),
            }
        }
        Ok(result)
    }

}

#[cfg(feature = "regex")]
impl std::fmt::Display for RegExpFlags {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.global { f.write_str("g")?; }
        if self.ignore_case { f.write_str("i")?; }
        if self.multiline { f.write_str("m")?; }
        if self.dot_all { f.write_str("s")?; }
        if self.unicode { f.write_str("u")?; }
        if self.sticky { f.write_str("y")?; }
        Ok(())
    }
}

/// Tish RegExp object
#[cfg(feature = "regex")]
#[derive(Debug, Clone)]
pub struct TishRegExp {
    pub source: String,
    pub flags: RegExpFlags,
    pub regex: Arc<Regex>,
    pub last_index: usize,
}

#[cfg(feature = "regex")]
impl TishRegExp {
    pub fn new(pattern: &str, flags_str: &str) -> Result<Self, String> {
        let flags = RegExpFlags::from_string(flags_str)?;
        let mut regex_pattern = pattern.to_string();
        
        if flags.ignore_case || flags.multiline || flags.dot_all {
            let mut flag_prefix = String::from("(?");
            if flags.ignore_case { flag_prefix.push('i'); }
            if flags.multiline { flag_prefix.push('m'); }
            if flags.dot_all { flag_prefix.push('s'); }
            flag_prefix.push(')');
            regex_pattern = format!("{}{}", flag_prefix, regex_pattern);
        }
        
        let regex = Regex::new(&regex_pattern)
            .map_err(|e| format!("Invalid regular expression: {}", e))?;
        
        Ok(Self { source: pattern.to_string(), flags, regex: Arc::new(regex), last_index: 0 })
    }

    pub fn flags_string(&self) -> String { self.flags.to_string() }

    pub fn test(&mut self, input: &str) -> bool {
        if self.flags.global || self.flags.sticky {
            let start = self.last_index;
            if start > input.chars().count() {
                self.last_index = 0;
                return false;
            }
            
            let byte_start: usize = input.chars().take(start).map(|c| c.len_utf8()).sum();
            let search_str = &input[byte_start..];
            
            match self.regex.find(search_str) {
                Ok(Some(m)) => {
                    if self.flags.sticky && m.start() != 0 {
                        self.last_index = 0;
                        return false;
                    }
                    let match_end_chars = input[byte_start..byte_start + m.end()].chars().count();
                    self.last_index = start + match_end_chars;
                    true
                }
                _ => {
                    self.last_index = 0;
                    false
                }
            }
        } else {
            self.regex.is_match(input).unwrap_or(false)
        }
    }
}

/// Runtime value for Tish programs.
/// Used by both interpreter and compiled code.
#[derive(Clone)]
pub enum Value {
    Number(f64),
    String(Arc<str>),
    Bool(bool),
    Null,
    Array(Rc<RefCell<Vec<Value>>>),
    Object(Rc<RefCell<HashMap<Arc<str>, Value>>>),
    Function(NativeFn),
    #[cfg(feature = "regex")]
    RegExp(Rc<RefCell<TishRegExp>>),
    /// Promise (for native compile). Interpreter uses tish_eval::Value::Promise.
    Promise(Arc<dyn TishPromise>),
    /// Opaque handle to a native Rust type (e.g. Polars DataFrame).
    Opaque(Arc<dyn TishOpaque>),
}

impl std::fmt::Debug for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Number(n) => write!(f, "Number({})", n),
            Value::String(s) => write!(f, "String({:?})", s.as_ref()),
            Value::Bool(b) => write!(f, "Bool({})", b),
            Value::Null => write!(f, "Null"),
            Value::Array(arr) => write!(f, "Array({:?})", arr.borrow()),
            Value::Object(obj) => write!(f, "Object({:?})", obj.borrow()),
            Value::Function(_) => write!(f, "Function"),
            #[cfg(feature = "regex")]
            Value::RegExp(re) => write!(f, "RegExp(/{}/{})", re.borrow().source, re.borrow().flags_string()),
            Value::Promise(_) => write!(f, "Promise"),
            Value::Opaque(o) => write!(f, "{}(opaque)", o.type_name()),
        }
    }
}

impl Value {
    /// Convert value to display string (for console output).
    pub fn to_display_string(&self) -> String {
        match self {
            Value::Number(n) => {
                if n.is_nan() {
                    "NaN".to_string()
                } else if *n == f64::INFINITY {
                    "Infinity".to_string()
                } else if *n == f64::NEG_INFINITY {
                    "-Infinity".to_string()
                } else {
                    n.to_string()
                }
            }
            Value::String(s) => s.to_string(),
            Value::Bool(b) => b.to_string(),
            Value::Null => "null".to_string(),
            Value::Array(arr) => {
                let inner: Vec<String> = arr.borrow().iter().map(|v| v.to_display_string()).collect();
                format!("[{}]", inner.join(", "))
            }
            Value::Object(obj) => {
                let inner: Vec<String> = obj
                    .borrow()
                    .iter()
                    .map(|(k, v)| format!("{}: {}", k.as_ref(), v.to_display_string()))
                    .collect();
                format!("{{{}}}", inner.join(", "))
            }
            Value::Function(_) => "[Function]".to_string(),
            Value::Promise(_) => "[object Promise]".to_string(),
            Value::Opaque(o) => format!("[object {}]", o.type_name()),
            #[cfg(feature = "regex")]
            Value::RegExp(re) => {
                let re = re.borrow();
                format!("/{}/{}", re.source, re.flags_string())
            }
        }
    }

    /// Check if value is truthy (for conditionals).
    pub fn is_truthy(&self) -> bool {
        match self {
            Value::Null => false,
            Value::Bool(b) => *b,
            Value::Number(n) => *n != 0.0 && !n.is_nan(),
            Value::String(s) => !s.is_empty(),
            _ => true,
        }
    }

    /// Strict equality (===).
    pub fn strict_eq(&self, other: &Value) -> bool {
        match (self, other) {
            (Value::Number(a), Value::Number(b)) => {
                if a.is_nan() || b.is_nan() {
                    false
                } else {
                    a == b
                }
            }
            (Value::String(a), Value::String(b)) => a == b,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Null, Value::Null) => true,
            (Value::Array(a), Value::Array(b)) => Rc::ptr_eq(a, b),
            (Value::Object(a), Value::Object(b)) => Rc::ptr_eq(a, b),
            (Value::Function(a), Value::Function(b)) => Rc::ptr_eq(a, b),
            #[cfg(feature = "regex")]
            (Value::RegExp(a), Value::RegExp(b)) => Rc::ptr_eq(a, b),
            (Value::Promise(a), Value::Promise(b)) => Arc::ptr_eq(a, b),
            (Value::Opaque(a), Value::Opaque(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }

    /// Create a new array Value from a Vec.
    pub fn array(items: Vec<Value>) -> Self {
        Value::Array(Rc::new(RefCell::new(items)))
    }

    /// Create a new object Value from a HashMap.
    pub fn object(map: HashMap<Arc<str>, Value>) -> Self {
        Value::Object(Rc::new(RefCell::new(map)))
    }

    /// Create an empty array Value.
    pub fn empty_array() -> Self {
        Value::Array(Rc::new(RefCell::new(Vec::new())))
    }

    /// Create an empty object Value.
    pub fn empty_object() -> Self {
        Value::Object(Rc::new(RefCell::new(HashMap::new())))
    }

    /// Extract the number value, if this is a Number.
    pub fn as_number(&self) -> Option<f64> {
        match self {
            Value::Number(n) => Some(*n),
            _ => None,
        }
    }

    /// JavaScript-style typeof string for this value.
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Number(_) => "number",
            Value::String(_) => "string",
            Value::Bool(_) => "boolean",
            Value::Null => "null",
            Value::Array(_) => "object",
            Value::Object(_) => "object",
            Value::Function(_) => "function",
            #[cfg(feature = "regex")]
            Value::RegExp(_) => "object",
            Value::Promise(_) => "object",
            Value::Opaque(o) => o.type_name(),
        }
    }
}
