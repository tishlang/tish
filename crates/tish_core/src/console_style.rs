//! Console styling for values (Node/Bun-style colors).
//!
//! Use for REPL, console.log, and any terminal output so numbers, strings,
//! booleans, null, and object structure are easier to scan.

use std::io::IsTerminal;

use crate::Value;

/// ANSI escape codes (standard 4-bit + bright black for dim).
const RESET: &str = "\x1b[0m";
/// Number: yellow (Node-style)
const NUMBER: &str = "\x1b[33m";
/// String: green
const STRING: &str = "\x1b[32m";
/// Boolean: blue
const BOOLEAN: &str = "\x1b[34m";
/// Null: dim grey
const NULL: &str = "\x1b[90m";
/// Object keys: cyan
const KEY: &str = "\x1b[36m";
/// Punctuation (brackets, commas): dim
const PUNCT: &str = "\x1b[90m";
/// Function / special (e.g. [Function]): dim
const SPECIAL: &str = "\x1b[90m";

/// Returns whether console output should use colors (stdout is a TTY).
pub fn use_console_colors() -> bool {
    std::io::stdout().is_terminal()
}

/// Format a single value for console with optional ANSI colors (Node/Bun-style).
pub fn format_value_styled(value: &Value, colors: bool) -> String {
    if !colors {
        return value.to_display_string();
    }
    format_value_styled_inner(value, colors)
}

fn format_value_styled_inner(value: &Value, colors: bool) -> String {
    match value {
        Value::Number(n) => {
            let s = if n.is_nan() {
                "NaN".to_string()
            } else if *n == f64::INFINITY {
                "Infinity".to_string()
            } else if *n == f64::NEG_INFINITY {
                "-Infinity".to_string()
            } else {
                n.to_string()
            };
            format!("{NUMBER}{s}{RESET}")
        }
        Value::String(s) => {
            let escaped = escape_string_for_display(s);
            format!("{STRING}\"{escaped}\"{RESET}")
        }
        Value::Bool(b) => format!("{BOOLEAN}{b}{RESET}"),
        Value::Null => format!("{NULL}null{RESET}"),
        Value::Array(arr) => {
            let inner: Vec<String> = arr
                .borrow()
                .iter()
                .map(|v| format_value_styled_inner(v, colors))
                .collect();
            let sep = format!("{PUNCT}, {RESET}");
            format!("{PUNCT}[{RESET}{}{PUNCT}]{RESET}", inner.join(&sep))
        }
        Value::Object(obj) => {
            let inner: Vec<String> = obj
                .borrow()
                .iter()
                .map(|(k, v)| {
                    format!(
                        "{KEY}{}{RESET}{PUNCT}: {RESET}{}",
                        k.as_ref(),
                        format_value_styled_inner(v, colors)
                    )
                })
                .collect();
            let sep = format!("{PUNCT}, {RESET}");
            format!("{PUNCT}{{{RESET} {} {PUNCT}}}{RESET}", inner.join(&sep))
        }
        Value::Function(_) => format!("{SPECIAL}[Function]{RESET}"),
        Value::Promise(_) => format!("{SPECIAL}[object Promise]{RESET}"),
        Value::Opaque(o) => format!("{SPECIAL}[object {}]{RESET}", o.type_name()),
        #[cfg(feature = "regex")]
        Value::RegExp(re) => {
            let re = re.borrow();
            format!(
                "{PUNCT}/{KEY}{}{RESET}{PUNCT}/{}{RESET}",
                re.source,
                re.flags_string()
            )
        }
    }
}

fn escape_string_for_display(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '"' => out.push_str("\\\""),
            c => out.push(c),
        }
    }
    out
}

/// Format multiple values for console (e.g. console.log(a, b, c)) with optional colors.
pub fn format_values_for_console(values: &[Value], colors: bool) -> String {
    let mut iter = values.iter();
    match iter.next() {
        None => String::new(),
        Some(first) => {
            let mut result = format_value_styled(first, colors);
            for v in iter {
                result.push(' ');
                result.push_str(&format_value_styled(v, colors));
            }
            result
        }
    }
}
