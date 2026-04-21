//! Built-in and JSX-intrinsic "go to definition" for the Tish LSP.
//!
//! Global / namespace member anchors are loaded from `stdlib/builtins.d.tish` in the `tish`
//! repository via `// @tish-source <symbol> <rel-path> <1-based-line>` pragmas. The type
//! surface in that file is the canonical declaration for ambient builtins.
//!
//! HTML / SVG intrinsic tag names are still listed here (sorted) for fast lookup; each maps
//! to the same vnode factory as in the pragma table for `div`.
//!
//! Definitions resolve to `file://` URIs only when `TISHLANG_SOURCE_ROOT` is set or the client
//! passes `tishlangSourceRoot` in LSP `initializationOptions` (VS Code: `tish.tishlangSourceRoot`).

use std::collections::HashMap;
use std::path::Path;
use std::sync::OnceLock;

use regex::Regex;
use tower_lsp::lsp_types::{Location, Position, Range, Url};

/// Stable path relative to the `tish` repository root (where the workspace `Cargo.toml` lives).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuiltinDef {
    pub rel_path: String,
    /// 0-based LSP line in the target file.
    pub line: u32,
    /// 0-based UTF-16 code unit offset on that line (ASCII-only targets use byte column).
    pub character: u32,
}

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// HTML / SVG intrinsic tag names (lowercase) that JSX lowers to `h("tag", …)`; must stay sorted for `binary_search`.
const HTML_INTRINSIC_TAGS: &[&str] = &[
    "a", "abbr", "address", "area", "article", "aside", "audio", "b", "base", "bdi", "bdo",
    "blockquote", "body", "br", "button", "canvas", "caption", "cite", "code", "col", "colgroup",
    "data", "datalist", "dd", "del", "details", "dfn", "dialog", "div", "dl", "dt", "em", "embed",
    "fieldset", "figcaption", "figure", "footer", "form", "h1", "h2", "h3", "h4", "h5", "h6",
    "head", "header", "hr", "html", "i", "iframe", "img", "input", "ins", "kbd", "label", "legend",
    "li", "link", "main", "map", "mark", "meta", "meter", "nav", "noscript", "object", "ol",
    "optgroup", "option", "output", "p", "param", "picture", "pre", "progress", "q", "rp", "rt",
    "ruby", "s", "samp", "script", "section", "select", "slot", "small", "source", "span", "strong",
    "style", "sub", "summary", "sup", "svg", "table", "tbody", "td", "template", "textarea",
    "tfoot", "th", "thead", "time", "title", "tr", "track", "u", "ul", "var", "video", "wbr",
];

static SOURCE_MAP: OnceLock<HashMap<String, BuiltinDef>> = OnceLock::new();

/// Parse `// @tish-source <symbol> <rel-path> <1-based-line>` lines (same format as `stdlib/builtins.d.tish`).
pub fn parse_tish_source_pragmas(src: &str) -> HashMap<String, BuiltinDef> {
    let re = Regex::new(r"(?m)^\s*//\s*@tish-source\s+(\S+)\s+(\S+)\s+(\d+)\s*$")
        .expect("builtin pragma regex");
    let mut m = HashMap::new();
    for cap in re.captures_iter(src) {
        let sym = cap[1].to_string();
        let rel = cap[2].to_string();
        let line_1: u32 = cap[3].parse().unwrap_or(1);
        m.insert(
            sym,
            BuiltinDef {
                rel_path: rel,
                line: line_1.saturating_sub(1),
                character: 0,
            },
        );
    }
    m
}

fn source_map() -> &'static HashMap<String, BuiltinDef> {
    SOURCE_MAP.get_or_init(|| {
        let src = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../stdlib/builtins.d.tish"
        ));
        parse_tish_source_pragmas(src)
    })
}

/// If `col` lies on the name of a JSX opening (or closing) tag on this line, returns the span of that name in **character indices** (same convention as [`tower_lsp::lsp_types::Position::character`] for ASCII lines).
fn jsx_tag_name_char_span(line: &str, col: usize) -> Option<(usize, usize)> {
    let chars: Vec<char> = line.chars().collect();
    if chars.is_empty() {
        return None;
    }
    let col = col.min(chars.len().saturating_sub(1));
    let mut j = col;
    while j > 0 {
        j -= 1;
        if chars[j] == '>' {
            return None;
        }
        if chars[j] == '<' {
            let mut k = j + 1;
            while k < chars.len() && chars[k].is_whitespace() {
                k += 1;
            }
            if k < chars.len() && chars[k] == '/' {
                k += 1;
                while k < chars.len() && chars[k].is_whitespace() {
                    k += 1;
                }
            }
            let name_start = k;
            while k < chars.len() {
                let c = chars[k];
                if c.is_whitespace() || c == '>' || c == '/' || c == '{' {
                    break;
                }
                if !(c.is_alphanumeric() || c == '_' || c == '-') {
                    break;
                }
                k += 1;
            }
            let name_end = k;
            if name_start < name_end && col >= name_start && col < name_end {
                return Some((name_start, name_end));
            }
            return None;
        }
    }
    None
}

fn tag_name_at_span(line: &str, start: usize, end: usize) -> String {
    line.chars()
        .enumerate()
        .filter(|(i, _)| *i >= start && *i < end)
        .map(|(_, c)| c)
        .collect()
}

/// `base.member` when the cursor is on `member` (same line, character-index column as `word_at_position`).
fn split_property_access(line: &str, col: usize) -> Option<(String, String)> {
    let chars: Vec<char> = line.chars().collect();
    if chars.is_empty() {
        return None;
    }
    let col = col.min(chars.len().saturating_sub(1));
    if !is_ident_char(chars[col]) {
        return None;
    }
    let mut end = col;
    while end + 1 < chars.len() && is_ident_char(chars[end + 1]) {
        end += 1;
    }
    let mut start = col;
    while start > 0 && is_ident_char(chars[start - 1]) {
        start -= 1;
    }
    if start == 0 {
        return None;
    }
    if chars[start - 1] != '.' {
        return None;
    }
    let member: String = chars[start..=end].iter().collect();
    let mut k = start - 2;
    while k > 0 && is_ident_char(chars[k - 1]) {
        k -= 1;
    }
    let base: String = chars[k..start - 1].iter().collect();
    if base.is_empty() {
        return None;
    }
    Some((base, member))
}

fn lookup_dotted(base: &str, member: &str) -> Option<BuiltinDef> {
    let key = format!("{base}.{member}");
    source_map().get(&key).cloned()
}

fn lookup_global(word: &str) -> Option<BuiltinDef> {
    source_map().get(word).cloned()
}

/// Built-in or JSX-intrinsic definition for the identifier at `position`, if any.
pub fn definition_for_builtin(
    text: &str,
    line: u32,
    character: u32,
    word: &str,
) -> Option<BuiltinDef> {
    let line_str = text.lines().nth(line as usize)?;
    let col = character as usize;

    if let Some((ns, ne)) = jsx_tag_name_char_span(line_str, col) {
        let tag = tag_name_at_span(line_str, ns, ne);
        if tag == word {
            if word == "Fragment" {
                return lookup_global("Fragment");
            }
            if HTML_INTRINSIC_TAGS.binary_search(&word).is_ok() {
                // Intrinsic tags share the same `ui_h` entry point as `div`.
                return lookup_global("div");
            }
        }
    }

    if let Some((base, member)) = split_property_access(line_str, col) {
        if let Some(d) = lookup_dotted(base.as_str(), member.as_str()) {
            return Some(d);
        }
    }

    lookup_global(word)
}

pub fn to_file_location(root: &Path, def: &BuiltinDef) -> Option<Location> {
    let path = root.join(&def.rel_path);
    if !path.is_file() {
        return None;
    }
    let uri = Url::from_file_path(&path).ok()?;
    let p = Position {
        line: def.line,
        character: def.character,
    };
    Some(Location {
        uri,
        range: Range { start: p, end: p },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pragma_map_loads_console_log() {
        let d = lookup_global("console").expect("console");
        assert!(d.rel_path.contains("eval.rs"));
        let m = lookup_dotted("console", "log").expect("console.log");
        assert!(m.rel_path.contains("natives.rs"));
    }

    #[test]
    fn jsx_div_maps_to_ui_h() {
        let text = "let x = <div />";
        let line = 0u32;
        let defn = definition_for_builtin(text, line, 9, "div").expect("div builtin");
        assert!(defn.rel_path.contains("runtime"));
        assert_eq!(defn.line, 40);
    }

    #[test]
    fn set_timeout_global() {
        let defn =
            definition_for_builtin("setTimeout(0, fn() {})\n", 0, 0, "setTimeout").expect("timer");
        assert_eq!(defn.rel_path, "crates/tish_eval/src/timers.rs");
    }

    #[test]
    fn console_log_qualified() {
        let text = "console.log(1)";
        let defn = definition_for_builtin(text, 0, 10, "log").expect("console.log");
        assert_eq!(defn.rel_path, "crates/tish_eval/src/natives.rs");
    }
}
