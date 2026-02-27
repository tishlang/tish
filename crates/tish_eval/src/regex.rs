//! JavaScript-compatible regular expression support for Tish.
//!
//! Uses fancy-regex for JS-like features (lookahead/lookbehind, named groups).

use std::cell::RefCell;
use std::rc::Rc;

use fancy_regex::Regex;

use crate::value::Value;

/// JavaScript RegExp flags
#[derive(Debug, Clone, Default)]
pub struct RegExpFlags {
    pub global: bool,      // g - find all matches
    pub ignore_case: bool, // i - case insensitive
    pub multiline: bool,   // m - ^ and $ match line boundaries
    pub dot_all: bool,     // s - . matches newlines
    pub unicode: bool,     // u - unicode mode
    pub sticky: bool,      // y - sticky mode (match at lastIndex)
}

impl RegExpFlags {
    pub fn from_string(flags: &str) -> Result<Self, String> {
        let mut result = Self::default();
        for c in flags.chars() {
            match c {
                'g' => {
                    if result.global {
                        return Err(format!("Invalid flags: duplicate flag '{}'", c));
                    }
                    result.global = true;
                }
                'i' => {
                    if result.ignore_case {
                        return Err(format!("Invalid flags: duplicate flag '{}'", c));
                    }
                    result.ignore_case = true;
                }
                'm' => {
                    if result.multiline {
                        return Err(format!("Invalid flags: duplicate flag '{}'", c));
                    }
                    result.multiline = true;
                }
                's' => {
                    if result.dot_all {
                        return Err(format!("Invalid flags: duplicate flag '{}'", c));
                    }
                    result.dot_all = true;
                }
                'u' => {
                    if result.unicode {
                        return Err(format!("Invalid flags: duplicate flag '{}'", c));
                    }
                    result.unicode = true;
                }
                'y' => {
                    if result.sticky {
                        return Err(format!("Invalid flags: duplicate flag '{}'", c));
                    }
                    result.sticky = true;
                }
                _ => return Err(format!("Invalid flags: unknown flag '{}'", c)),
            }
        }
        Ok(result)
    }

    pub fn to_string(&self) -> String {
        let mut s = String::new();
        if self.global {
            s.push('g');
        }
        if self.ignore_case {
            s.push('i');
        }
        if self.multiline {
            s.push('m');
        }
        if self.dot_all {
            s.push('s');
        }
        if self.unicode {
            s.push('u');
        }
        if self.sticky {
            s.push('y');
        }
        s
    }
}

/// Tish RegExp object - wraps a compiled regex with JS semantics
#[derive(Debug)]
pub struct TishRegExp {
    pub source: String,
    pub flags: RegExpFlags,
    pub regex: Regex,
    pub last_index: usize,
}

impl Clone for TishRegExp {
    fn clone(&self) -> Self {
        Self {
            source: self.source.clone(),
            flags: self.flags.clone(),
            regex: Regex::new(self.regex.as_str()).unwrap(),
            last_index: self.last_index,
        }
    }
}

impl TishRegExp {
    /// Create a new RegExp from pattern and flags
    pub fn new(pattern: &str, flags_str: &str) -> Result<Self, String> {
        let flags = RegExpFlags::from_string(flags_str)?;
        
        // Build the pattern with inline flags for fancy-regex
        let mut regex_pattern = pattern.to_string();
        
        // Add inline flags prefix for i, m, s
        if flags.ignore_case || flags.multiline || flags.dot_all {
            let mut flag_prefix = String::from("(?");
            if flags.ignore_case {
                flag_prefix.push('i');
            }
            if flags.multiline {
                flag_prefix.push('m');
            }
            if flags.dot_all {
                flag_prefix.push('s');
            }
            flag_prefix.push(')');
            regex_pattern = format!("{}{}", flag_prefix, regex_pattern);
        }
        
        let regex = Regex::new(&regex_pattern)
            .map_err(|e| format!("Invalid regular expression: {}", e))?;
        
        Ok(Self {
            source: pattern.to_string(),
            flags,
            regex,
            last_index: 0,
        })
    }

    /// Returns the flags as a string (in canonical order: gimsuvy)
    pub fn flags_string(&self) -> String {
        self.flags.to_string()
    }

    /// RegExp.prototype.test(string) - returns true if pattern matches
    pub fn test(&mut self, input: &str) -> bool {
        if self.flags.global || self.flags.sticky {
            let start = self.last_index;
            if start > input.chars().count() {
                self.last_index = 0;
                return false;
            }
            
            // Get byte offset for start position
            let byte_start: usize = input.chars().take(start).map(|c| c.len_utf8()).sum();
            let search_str = &input[byte_start..];
            
            match self.regex.find(search_str) {
                Ok(Some(m)) => {
                    if self.flags.sticky && m.start() != 0 {
                        self.last_index = 0;
                        return false;
                    }
                    // Update lastIndex to end of match (in characters)
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

    /// RegExp.prototype.exec(string) - returns match array or null
    pub fn exec(&mut self, input: &str) -> Value {
        let start = if self.flags.global || self.flags.sticky {
            self.last_index
        } else {
            0
        };

        let char_count = input.chars().count();
        if start > char_count {
            if self.flags.global || self.flags.sticky {
                self.last_index = 0;
            }
            return Value::Null;
        }

        // Get byte offset for start position
        let byte_start: usize = input.chars().take(start).map(|c| c.len_utf8()).sum();
        let search_str = &input[byte_start..];

        match self.regex.captures(search_str) {
            Ok(Some(caps)) => {
                let full_match = caps.get(0).unwrap();
                
                // For sticky mode, match must start at position 0 of search_str
                if self.flags.sticky && full_match.start() != 0 {
                    self.last_index = 0;
                    return Value::Null;
                }

                // Build result array
                let mut result = Vec::new();
                
                // Add full match
                result.push(Value::String(full_match.as_str().into()));
                
                // Add capture groups
                for i in 1..caps.len() {
                    match caps.get(i) {
                        Some(m) => result.push(Value::String(m.as_str().into())),
                        None => result.push(Value::Null),
                    }
                }

                // Calculate match index in characters (from start of original string)
                let _match_start_chars = input[..byte_start + full_match.start()].chars().count();
                
                // Update lastIndex for global/sticky
                if self.flags.global || self.flags.sticky {
                    let match_end_chars = input[..byte_start + full_match.end()].chars().count();
                    // Prevent infinite loops on zero-length matches
                    if full_match.start() == full_match.end() {
                        self.last_index = match_end_chars + 1;
                    } else {
                        self.last_index = match_end_chars;
                    }
                }

                // Return as array (full JS semantics would add index and input properties)
                Value::Array(Rc::new(RefCell::new(result)))
            }
            Ok(None) => {
                if self.flags.global || self.flags.sticky {
                    self.last_index = 0;
                }
                Value::Null
            }
            Err(_) => {
                if self.flags.global || self.flags.sticky {
                    self.last_index = 0;
                }
                Value::Null
            }
        }
    }
}

/// Create a RegExp Value from pattern and flags
pub fn create_regexp(pattern: &str, flags: &str) -> Result<Value, String> {
    let re = TishRegExp::new(pattern, flags)?;
    Ok(Value::RegExp(Rc::new(RefCell::new(re))))
}

/// RegExp constructor function - handles `new RegExp(pattern, flags)` or `RegExp(pattern, flags)`
pub fn regexp_constructor(args: &[Value]) -> Result<Value, String> {
    let pattern = match args.first() {
        Some(Value::String(s)) => s.to_string(),
        Some(Value::RegExp(re)) => {
            // If first arg is RegExp and no flags provided, return a copy
            if args.get(1).is_none() {
                let re = re.borrow();
                return create_regexp(&re.source, &re.flags_string());
            }
            re.borrow().source.clone()
        }
        Some(v) => v.to_string(),
        None => String::new(),
    };

    let flags = match args.get(1) {
        Some(Value::String(s)) => s.to_string(),
        Some(Value::Null) | None => String::new(),
        Some(v) => v.to_string(),
    };

    create_regexp(&pattern, &flags)
}

// ============== String methods with regex support ==============

/// String.prototype.match(regexp) - returns array of matches or null
pub fn string_match(input: &str, regexp: &Value) -> Value {
    match regexp {
        Value::RegExp(re) => {
            let mut re = re.borrow_mut();
            
            if re.flags.global {
                // Global: return array of all matches (no capture groups)
                let mut matches = Vec::new();
                re.last_index = 0;
                
                loop {
                    match re.regex.find_from_pos(input, re.last_index) {
                        Ok(Some(m)) => {
                            matches.push(Value::String(m.as_str().into()));
                            // Prevent infinite loop on zero-length match
                            if m.start() == m.end() {
                                re.last_index = m.end() + 1;
                            } else {
                                re.last_index = m.end();
                            }
                            if re.last_index > input.len() {
                                break;
                            }
                        }
                        _ => break,
                    }
                }
                
                re.last_index = 0;
                
                if matches.is_empty() {
                    Value::Null
                } else {
                    Value::Array(Rc::new(RefCell::new(matches)))
                }
            } else {
                // Non-global: return first match with captures (like exec)
                re.exec(input)
            }
        }
        Value::String(pattern) => {
            // If passed a string, convert to regex
            match TishRegExp::new(pattern, "") {
                Ok(mut re) => re.exec(input),
                Err(_) => Value::Null,
            }
        }
        _ => Value::Null,
    }
}

/// String.prototype.replace(searchValue, replaceValue)
pub fn string_replace(input: &str, search: &Value, replace: &Value) -> Value {
    let replacement = match replace {
        Value::String(s) => s.to_string(),
        v => v.to_string(),
    };

    match search {
        Value::RegExp(re) => {
            let re = re.borrow();
            
            if re.flags.global {
                // Replace all matches
                match re.regex.replace_all(input, replacement.as_str()) {
                    std::borrow::Cow::Borrowed(s) => Value::String(s.into()),
                    std::borrow::Cow::Owned(s) => Value::String(s.into()),
                }
            } else {
                // Replace first match only
                match re.regex.replace(input, replacement.as_str()) {
                    std::borrow::Cow::Borrowed(s) => Value::String(s.into()),
                    std::borrow::Cow::Owned(s) => Value::String(s.into()),
                }
            }
        }
        Value::String(pattern) => {
            // Simple string replacement (first occurrence only)
            Value::String(input.replacen(pattern.as_ref(), &replacement, 1).into())
        }
        _ => Value::String(input.into()),
    }
}

/// String.prototype.search(regexp) - returns index of first match or -1
pub fn string_search(input: &str, regexp: &Value) -> Value {
    match regexp {
        Value::RegExp(re) => {
            let re = re.borrow();
            match re.regex.find(input) {
                Ok(Some(m)) => {
                    // Convert byte index to char index
                    let char_index = input[..m.start()].chars().count();
                    Value::Number(char_index as f64)
                }
                _ => Value::Number(-1.0),
            }
        }
        Value::String(pattern) => {
            match TishRegExp::new(pattern, "") {
                Ok(re) => match re.regex.find(input) {
                    Ok(Some(m)) => {
                        let char_index = input[..m.start()].chars().count();
                        Value::Number(char_index as f64)
                    }
                    _ => Value::Number(-1.0),
                },
                Err(_) => Value::Number(-1.0),
            }
        }
        _ => Value::Number(-1.0),
    }
}

/// String.prototype.split(separator, limit) - split string by regex or string
pub fn string_split(input: &str, separator: &Value, limit: Option<usize>) -> Value {
    let max = limit.unwrap_or(usize::MAX);
    
    if max == 0 {
        return Value::Array(Rc::new(RefCell::new(Vec::new())));
    }

    match separator {
        Value::RegExp(re) => {
            let re = re.borrow();
            let mut result = Vec::new();
            let mut last_end = 0;
            
            for mat in re.regex.find_iter(input) {
                match mat {
                    Ok(m) => {
                        if result.len() >= max - 1 {
                            break;
                        }
                        result.push(Value::String(input[last_end..m.start()].into()));
                        last_end = m.end();
                    }
                    Err(_) => break,
                }
            }
            
            // Add remaining part
            if result.len() < max {
                result.push(Value::String(input[last_end..].into()));
            }
            
            Value::Array(Rc::new(RefCell::new(result)))
        }
        Value::String(sep) => {
            let parts: Vec<Value> = input
                .splitn(max, sep.as_ref())
                .map(|s| Value::String(s.into()))
                .collect();
            Value::Array(Rc::new(RefCell::new(parts)))
        }
        Value::Null => {
            // null separator returns array with original string
            Value::Array(Rc::new(RefCell::new(vec![Value::String(input.into())])))
        }
        _ => {
            // Other types: convert to string and split
            let sep_str = separator.to_string();
            let parts: Vec<Value> = input
                .splitn(max, &sep_str)
                .map(|s| Value::String(s.into()))
                .collect();
            Value::Array(Rc::new(RefCell::new(parts)))
        }
    }
}
