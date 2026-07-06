//! String builtin methods.
//!
//! All indices use character (Unicode scalar) positions for consistency with
//! JavaScript, matching .length and .charAt(). Byte offsets are never exposed.

use crate::helpers::normalize_index;
use std::cell::RefCell;
use tishlang_core::ArcStr;
use tishlang_core::Value;
use tishlang_core::VmRef;

// #203: a per-thread cursor cache that makes repeated character indexing (`charCodeAt(i)`, `s[i]`,
// `charAt(i)`) O(1)/near-O(1) instead of O(i). tish strings are UTF-8, so `chars().nth(i)` scans from
// the start — turning indexed/strided scans into O(n^2) (a strided checksum over a 1.3 MB string was
// 4939ms vs node 1ms). For each recently-indexed string we cache whether it is all-ASCII (then a
// character index equals a byte index → O(1) byte lookup) plus a forward cursor (so non-ASCII
// sequential/strided scans advance from the last position, not from 0). Safety: the entry holds an
// `ArcStr` CLONE, which keeps the backing allocation alive — so its data pointer can't be freed and
// reused by another string while cached (no ABA), and since strings are immutable the cached ASCII
// flag stays valid. Backends share this via `char_at_idx` (native + VM route through the builtin).
// Semantics are unchanged: still character (Unicode scalar) indexing, identical to `chars().nth(i)`.
struct CharCursor {
    s: ArcStr,
    ascii: bool,
    /// Character (Unicode scalar) count, cached so `.length` is O(1) too — `for (i=0;i<s.length;i++)`
    /// re-evaluates the bound every iteration, so an O(n) `chars().count()` there is itself O(n^2).
    len_chars: usize,
    char_idx: usize,
    byte_off: usize,
}

thread_local! {
    static INDEX_CURSOR: RefCell<Option<CharCursor>> = const { RefCell::new(None) };
}

/// Borrow the cursor entry for `s`, reseeding (compute ASCII flag + character count) if it currently
/// caches a different backing allocation, then run `f` against it. Centralises the pointer-keyed
/// reseed shared by [`char_at_idx`] and [`char_count`].
fn with_cursor<R>(s: &ArcStr, f: impl FnOnce(&mut CharCursor, &ArcStr) -> R) -> R {
    INDEX_CURSOR.with(|cell| {
        let mut slot = cell.borrow_mut();
        let hit = matches!(slot.as_ref(), Some(c)
            if std::ptr::eq(c.s.as_bytes().as_ptr(), s.as_bytes().as_ptr()) && c.s.len() == s.len());
        if !hit {
            let ascii = s.as_bytes().is_ascii();
            // ASCII → character count equals byte length (free); otherwise count once.
            let len_chars = if ascii { s.len() } else { s.chars().count() };
            *slot = Some(CharCursor {
                s: s.clone(),
                ascii,
                len_chars,
                char_idx: 0,
                byte_off: 0,
            });
        }
        f(slot.as_mut().unwrap(), s)
    })
}

/// Character (Unicode scalar) count of `s`, O(1) after the first call on a given string.
pub fn char_count(s: &ArcStr) -> usize {
    with_cursor(s, |c, _| c.len_chars)
}

/// Byte offset -> character index.
fn byte_to_char_index(s: &str, byte_offset: usize) -> usize {
    s.char_indices()
        .take_while(|(i, _)| *i < byte_offset)
        .count()
}

/// Character index -> byte offset.
fn char_to_byte_offset(s: &str, char_index: usize) -> usize {
    s.char_indices()
        .nth(char_index)
        .map(|(i, _)| i)
        .unwrap_or(s.len())
}

/// Create a new string Value from a string slice.
pub fn from_str(s: &str) -> Value {
    Value::String(tishlang_core::ArcStr::from(s))
}

/// Get the length of a string (character count).
pub fn len(s: &Value) -> Option<usize> {
    match s {
        Value::String(str) => Some(char_count(str)),
        _ => None,
    }
}

/// JS `ToIntegerOrInfinity` then clamp for `lastIndexOf` `position` (character index).
fn last_index_of_position_to_start(position: &Value, len: usize) -> usize {
    let pos = match position {
        Value::Null => 0.0,
        Value::Bool(false) => 0.0,
        Value::Bool(true) => 1.0,
        Value::Number(n) => {
            if n.is_nan() || *n == 0.0 {
                0.0
            } else if n.is_infinite() {
                *n
            } else {
                n.trunc()
            }
        }
        _ => 0.0,
    };
    if pos.is_infinite() {
        if pos > 0.0 {
            len
        } else {
            0
        }
    } else if pos <= 0.0 {
        0
    } else {
        (pos as usize).min(len)
    }
}

/// Character index of last occurrence of `needle` in `haystack`, or `-1`.
/// `position` is JS `lastIndexOf`'s second argument: use `Number(INFINITY)` when omitted;
/// `Null` is JS `null` → 0. Indices are Unicode scalar positions (same as `.length` / `indexOf`).
pub fn last_index_of_str(haystack: &str, needle: &str, position: &Value) -> Value {
    let len = haystack.chars().count();
    let start = last_index_of_position_to_start(position, len);
    let hay: Vec<char> = haystack.chars().collect();
    let needle_chars: Vec<char> = needle.chars().collect();
    let search_len = needle_chars.len();
    if search_len == 0 {
        return Value::Number(start as f64);
    }
    if search_len > len {
        return Value::Number(-1.0);
    }
    // Match must fit in the string and end at or before `start` (ECMA `lastIndexOf` position).
    if start + 1 < search_len {
        return Value::Number(-1.0);
    }
    let k_max_by_len = len - search_len;
    let k_max_by_start = start + 1 - search_len;
    let k_max = k_max_by_len.min(k_max_by_start);
    let mut k = k_max;
    loop {
        if hay[k..k + search_len] == needle_chars[..] {
            return Value::Number(k as f64);
        }
        if k == 0 {
            break;
        }
        k -= 1;
    }
    Value::Number(-1.0)
}

/// Like [`last_index_of_str`] but takes string `Value`s; non-strings → `-1`.
pub fn last_index_of(s: &Value, search: &Value, position: &Value) -> Value {
    if let (Value::String(h), Value::String(n)) = (s, search) {
        last_index_of_str(h.as_ref(), n.as_ref(), position)
    } else {
        Value::Number(-1.0)
    }
}

/// Returns character index of first occurrence, or -1. Optional fromIndex (JS indexOf).
pub fn index_of(s: &Value, search: &Value, from: Option<&Value>) -> Value {
    if let (Value::String(s), Value::String(search)) = (s, search) {
        let from_char = match from {
            Some(Value::Number(n)) if *n >= 0.0 => (*n as usize).min(s.chars().count()),
            _ => 0,
        };
        let byte_start = char_to_byte_offset(s, from_char);
        let search_str = search.as_str();
        if let Some(byte_pos) = s[byte_start..].find(search_str) {
            let char_idx = from_char + byte_to_char_index(&s[byte_start..], byte_pos);
            Value::Number(char_idx as f64)
        } else {
            Value::Number(-1.0)
        }
    } else {
        Value::Number(-1.0)
    }
}

pub fn includes(s: &Value, search: &Value, from: Option<&Value>) -> Value {
    if let (Value::String(s), Value::String(search)) = (s, search) {
        let from_char = match from {
            Some(Value::Number(n)) if *n >= 0.0 => (*n as usize).min(s.chars().count()),
            Some(Value::Number(n)) if *n < 0.0 => {
                let len = s.chars().count() as i64;
                ((len + *n as i64).max(0)) as usize
            }
            _ => 0,
        };
        let byte_start = char_to_byte_offset(s, from_char);
        Value::Bool(s[byte_start..].contains(search.as_str()))
    } else {
        Value::Bool(false)
    }
}

pub fn slice(s: &Value, start: &Value, end: &Value) -> Value {
    if let Value::String(s) = s {
        let chars: Vec<char> = s.chars().collect();
        let len = chars.len() as i64;
        let (si, ei) = (
            normalize_index(start, len, 0),
            normalize_index(end, len, len as usize),
        );
        let result: String = if si < ei {
            chars[si..ei].iter().collect()
        } else {
            String::new()
        };
        Value::String(result.into())
    } else {
        Value::Null
    }
}

pub fn substring(s: &Value, start: &Value, end: &Value) -> Value {
    fn bounds(start: &Value, end: &Value, len: usize) -> (usize, usize) {
        let si = match start {
            Value::Number(n) => (*n as usize).min(len),
            _ => 0,
        };
        let ei = match end {
            Value::Null => len,
            Value::Number(n) => (*n as usize).min(len),
            _ => len,
        };
        (si.min(ei), si.max(ei))
    }
    if let Value::String(s) = s {
        let chars: Vec<char> = s.chars().collect();
        let (ss, ee) = bounds(start, end, chars.len());
        let result: String = chars[ss..ee].iter().collect();
        Value::String(result.into())
    } else {
        Value::Null
    }
}

/// JS `String.prototype.substr(start, length)`.
pub fn substr(s: &Value, start: &Value, length: &Value) -> Value {
    if let Value::String(s) = s {
        let chars: Vec<char> = s.chars().collect();
        let len = chars.len();
        let mut start_idx = match start {
            Value::Number(n) => *n as i64,
            _ => 0,
        };
        if start_idx < 0 {
            start_idx = (len as i64 + start_idx).max(0);
        }
        let start_idx = (start_idx as usize).min(len);
        let count = match length {
            Value::Null => len - start_idx,
            Value::Number(n) => (*n as i64).max(0) as usize,
            _ => len - start_idx,
        };
        let end_idx = (start_idx + count).min(len);
        let result: String = chars[start_idx..end_idx].iter().collect();
        Value::String(result.into())
    } else {
        Value::Null
    }
}

pub fn split(s: &Value, sep: &Value) -> Value {
    split_limit(s, sep, None)
}

/// `String.prototype.split(sep, limit)` for a string separator. JS semantics: the string is split
/// completely on `sep`, then the result is truncated to `limit` elements (it does NOT keep the
/// unsplit remainder in the last slot, which is what Rust's `splitn` would do). `limit == 0` yields
/// an empty array. This is the single source of truth shared by the VM and rust/cranelift/wasi
/// backends; the interpreter mirrors it in `tish_eval::regex::string_split`.
pub fn split_limit(s: &Value, sep: &Value, limit: Option<usize>) -> Value {
    if let Value::String(s) = s {
        let separator = match sep {
            Value::String(ss) => ss.as_str(),
            _ => return Value::Array(VmRef::new(vec![Value::String(s.clone())])),
        };
        if limit == Some(0) {
            return Value::Array(VmRef::new(Vec::new()));
        }
        // JS `split("")` is special: it yields the string's characters with NO surrounding empties
        // (`"xyz".split("")` → `["x","y","z"]`, `"".split("")` → `[]`), unlike Rust's `str::split("")`
        // which emits leading/trailing `""`. (tish splits on `char`s; lone-surrogate behavior for
        // astral code points is out of scope.) #247
        if separator.is_empty() {
            let mut parts: Vec<Value> =
                s.chars().map(|c| Value::String(c.to_string().into())).collect();
            if let Some(max) = limit {
                parts.truncate(max);
            }
            return Value::Array(VmRef::new(parts));
        }
        let mut parts: Vec<Value> = s.split(separator).map(|p| Value::String(p.into())).collect();
        if let Some(max) = limit {
            parts.truncate(max);
        }
        Value::Array(VmRef::new(parts))
    } else {
        Value::Null
    }
}

pub fn trim(s: &Value) -> Value {
    if let Value::String(s) = s {
        Value::String(s.trim().into())
    } else {
        Value::Null
    }
}

pub fn to_upper_case(s: &Value) -> Value {
    if let Value::String(s) = s {
        Value::String(s.to_uppercase().into())
    } else {
        Value::Null
    }
}

pub fn to_lower_case(s: &Value) -> Value {
    if let Value::String(s) = s {
        Value::String(s.to_lowercase().into())
    } else {
        Value::Null
    }
}

pub fn starts_with(s: &Value, search: &Value, position: Option<&Value>) -> Value {
    if let (Value::String(s), Value::String(search)) = (s, search) {
        // `position`: test as if the string began at char `position` (clamped to [0, len]). Absent → 0.
        let pos = match position {
            Some(Value::Number(n)) if *n > 0.0 => *n as usize,
            _ => 0,
        };
        let byte_start = s.char_indices().nth(pos).map(|(b, _)| b).unwrap_or(s.len());
        Value::Bool(s[byte_start..].starts_with(search.as_str()))
    } else {
        Value::Bool(false)
    }
}

pub fn ends_with(s: &Value, search: &Value, end_position: Option<&Value>) -> Value {
    if let (Value::String(s), Value::String(search)) = (s, search) {
        // `endPosition`: test as if the string ended at char `endPosition` (clamped to [0, len]).
        // Absent → the full length.
        let char_count = s.chars().count();
        let end = match end_position {
            Some(Value::Number(n)) if *n >= 0.0 => (*n as usize).min(char_count),
            Some(Value::Number(_)) => 0,
            _ => char_count,
        };
        let byte_end = s.char_indices().nth(end).map(|(b, _)| b).unwrap_or(s.len());
        Value::Bool(s[..byte_end].ends_with(search.as_str()))
    } else {
        Value::Bool(false)
    }
}

fn replace_impl(s: &Value, search: &Value, replacement: &Value, all: bool) -> Value {
    if let Value::String(s) = s {
        let search_str = match search {
            Value::String(ss) => ss.as_str(),
            _ => return Value::String(s.clone()),
        };
        let repl_str = match replacement {
            Value::String(ss) => ss.as_str(),
            _ => "",
        };
        let result = if all {
            s.replace(search_str, repl_str)
        } else {
            s.replacen(search_str, repl_str, 1)
        };
        Value::String(result.into())
    } else {
        Value::Null
    }
}

pub fn replace(s: &Value, search: &Value, replacement: &Value) -> Value {
    replace_impl(s, search, replacement, false)
}

pub fn replace_all(s: &Value, search: &Value, replacement: &Value) -> Value {
    replace_impl(s, search, replacement, true)
}

/// HTML entity escape for the five canonical characters (`& < > " '`).
/// Single linear pass over the input; takes a zero-copy fast path when no
/// character needs escaping. Matches TFB's fortunes verifier byte-for-byte.
pub fn escape_html(s: &Value) -> Value {
    let input = match s {
        Value::String(s) => s.as_str(),
        Value::Null => return Value::String(tishlang_core::ArcStr::from("")),
        _ => return Value::Null,
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
        return Value::String(match s {
            Value::String(s) => s.clone(),
            _ => unreachable!(),
        });
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
    Value::String(tishlang_core::ArcStr::from(out))
}

/// Character (Unicode scalar) at index `idx`, using the cursor cache (see [`CharCursor`]).
/// Equivalent to `s.chars().nth(idx)` but O(1) for ASCII strings and near-O(1) for forward/strided
/// scans of non-ASCII strings, instead of O(idx) every call.
fn char_at_idx(s: &ArcStr, idx: usize) -> Option<char> {
    with_cursor(s, |c, s| {
        if c.ascii {
            // ASCII: character index == byte index, and every byte is its own scalar.
            return s.as_bytes().get(idx).map(|&b| b as char);
        }
        // Non-ASCII: advance from the nearest known position (forward fast path); restart from 0 only
        // when indexing backwards relative to the cursor.
        let (base_idx, base_off) = if idx >= c.char_idx {
            (c.char_idx, c.byte_off)
        } else {
            (0, 0)
        };
        match s[base_off..].char_indices().nth(idx - base_idx) {
            Some((rel_off, ch)) => {
                c.char_idx = idx;
                c.byte_off = base_off + rel_off;
                Some(ch)
            }
            None => None,
        }
    })
}

/// Character (Unicode scalar) at index `idx` via the cursor cache — the O(1)/near-O(1) primitive
/// behind `s[i]`. Returns `None` for an out-of-range index; each backend maps that to its own
/// out-of-bounds behaviour (interpreter/native → null, VM → error).
pub fn nth_char(s: &ArcStr, idx: usize) -> Option<char> {
    char_at_idx(s, idx)
}

pub fn char_at(s: &Value, idx: &Value) -> Value {
    if let Value::String(s) = s {
        let idx = match idx {
            Value::Number(n) => *n as usize,
            _ => 0,
        };
        char_at_idx(s, idx)
            .map(|c| Value::String(c.to_string().into()))
            .unwrap_or(Value::String("".into()))
    } else {
        Value::Null
    }
}

/// `String.prototype.at(index)` — like `charAt` but negative `index` counts from the end and an
/// out-of-range index yields `null` (JS `undefined`), not `""`. #247
pub fn at(s: &Value, index: &Value) -> Value {
    if let Value::String(s) = s {
        let i = match index {
            Value::Number(n) => *n as i64,
            _ => 0,
        };
        // Non-negative indices use the cursor cache directly; a negative index counts from the end,
        // which needs the character length first (inherently O(n)).
        let idx = if i < 0 { s.chars().count() as i64 + i } else { i };
        if idx >= 0 {
            if let Some(c) = char_at_idx(s, idx as usize) {
                return Value::String(c.to_string().into());
            }
        }
    }
    Value::Null
}

pub fn char_code_at(s: &Value, idx: &Value) -> Value {
    if let Value::String(s) = s {
        let idx = match idx {
            Value::Number(n) => *n as usize,
            _ => 0,
        };
        char_at_idx(s, idx)
            .map(|c| Value::Number(c as u32 as f64))
            .unwrap_or(Value::Number(f64::NAN))
    } else {
        Value::Null
    }
}

pub fn repeat(s: &Value, count: &Value) -> Value {
    if let Value::String(s) = s {
        let count = match count {
            Value::Number(n) if *n >= 0.0 => *n as usize,
            _ => 0,
        };
        Value::String(s.repeat(count).into())
    } else {
        Value::Null
    }
}

fn pad_impl(s: &Value, target_len: &Value, pad: &Value, at_start: bool) -> Value {
    if let Value::String(s) = s {
        let target_len = match target_len {
            Value::Number(n) => *n as usize,
            _ => return Value::String(s.clone()),
        };
        // An *explicit* empty fill string means "no padding" (spec: `padStart(n, "")` → original,
        // unchanged); only an ABSENT pad arg (`Value::Null`) defaults to a space. The old
        // `if !p.is_empty()` guard conflated the two, space-padding on an explicit `""`.
        let pad_str = match pad {
            Value::String(p) => p.as_str(),
            _ => " ",
        };
        let char_count = s.chars().count();
        if char_count >= target_len || pad_str.is_empty() {
            return Value::String(s.clone());
        }
        let needed = target_len - char_count;
        let padding: String = pad_str.chars().cycle().take(needed).collect();
        let result = if at_start {
            format!("{}{}", padding, s)
        } else {
            format!("{}{}", s, padding)
        };
        Value::String(result.into())
    } else {
        Value::Null
    }
}

pub fn pad_start(s: &Value, target_len: &Value, pad: &Value) -> Value {
    pad_impl(s, target_len, pad, true)
}

pub fn pad_end(s: &Value, target_len: &Value, pad: &Value) -> Value {
    pad_impl(s, target_len, pad, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(x: &str) -> Value {
        Value::String(x.into())
    }

    fn n(x: f64) -> Value {
        Value::Number(x)
    }

    fn same(a: &Value, b: &Value) -> bool {
        match (a, b) {
            (Value::String(x), Value::String(y)) => x == y,
            (Value::Number(x), Value::Number(y)) => {
                if x.is_nan() && y.is_nan() {
                    true
                } else {
                    x == y
                }
            }
            (Value::Bool(x), Value::Bool(y)) => x == y,
            (Value::Null, Value::Null) => true,
            (Value::Array(ax), Value::Array(ay)) => {
                let bx = ax.borrow();
                let by = ay.borrow();
                bx.len() == by.len() && bx.iter().zip(by.iter()).all(|(u, v)| same(u, v))
            }
            _ => false,
        }
    }

    macro_rules! assert_same {
        ($left:expr, $right:expr) => {
            assert!(same(&$left, &$right), "left={:?} right={:?}", $left, $right);
        };
    }

    #[test]
    fn index_of_basic() {
        assert_same!(index_of(&s("abc"), &s("b"), None), n(1.0));
        assert_same!(index_of(&s("abc"), &s("x"), None), n(-1.0));
        assert_same!(index_of(&s("abca"), &s("a"), Some(&n(1.0))), n(3.0));
    }

    #[test]
    fn index_of_non_string() {
        assert_same!(index_of(&n(1.0), &s("a"), None), n(-1.0));
        assert_same!(index_of(&s("a"), &n(1.0), None), n(-1.0));
    }

    #[test]
    fn includes_basic() {
        assert_same!(includes(&s("hello"), &s("ll"), None), Value::Bool(true));
        assert_same!(includes(&s("hello"), &s("x"), None), Value::Bool(false));
        assert_same!(
            includes(&s("hello"), &s("l"), Some(&n(3.0))),
            Value::Bool(true)
        );
        assert_same!(
            includes(&s("hello"), &s("l"), Some(&n(4.0))),
            Value::Bool(false)
        );
    }

    #[test]
    fn includes_negative_from() {
        assert_same!(
            includes(&s("hello"), &s("o"), Some(&n(-1.0))),
            Value::Bool(true)
        );
        assert_same!(
            includes(&s("hello"), &s("h"), Some(&n(-5.0))),
            Value::Bool(true)
        );
        // fromIndex -1 → start at len-1 = 1 ("i" only), "h" not found
        assert_same!(
            includes(&s("hi"), &s("h"), Some(&n(-1.0))),
            Value::Bool(false)
        );
    }

    #[test]
    fn includes_non_string() {
        assert_same!(includes(&n(1.0), &s("a"), None), Value::Bool(false));
    }

    #[test]
    fn slice_substring() {
        assert_same!(slice(&s("hello"), &n(1.0), &n(4.0)), s("ell"));
        assert_same!(slice(&s("hello"), &n(-3.0), &Value::Null), s("llo"));
        assert_same!(substring(&s("hello"), &n(4.0), &n(1.0)), s("ell"));
        assert_same!(slice(&s("ab"), &n(1.0), &n(1.0)), s(""));
    }

    #[test]
    fn slice_non_string() {
        assert_same!(slice(&n(1.0), &n(0.0), &Value::Null), Value::Null);
    }

    #[test]
    fn split_trim() {
        let Value::Array(a) = split(&s("a,b"), &s(",")) else {
            panic!();
        };
        assert_eq!(a.borrow().len(), 2);
        assert_same!(
            split(&s("x"), &n(1.0)),
            Value::Array(VmRef::new(vec![s("x")]))
        );
        assert_same!(split(&n(1.0), &s(",")), Value::Null);
        assert_same!(trim(&s("  x  ")), s("x"));
        assert_same!(trim(&n(1.0)), Value::Null);
    }

    #[test]
    fn split_limit_js_semantics() {
        let parts = |v: &Value| -> Vec<String> {
            let Value::Array(a) = v else { panic!("not array") };
            a.borrow()
                .iter()
                .map(|x| match x {
                    Value::String(s) => s.to_string(),
                    _ => panic!("not string"),
                })
                .collect()
        };
        // limit truncates to the first N pieces (does NOT keep the remainder, unlike `splitn`)
        assert_eq!(parts(&split_limit(&s("a,b,c,d"), &s(","), Some(2))), ["a", "b"]);
        // limit 0 -> empty; limit beyond piece count -> full split; no limit -> full split
        assert_eq!(parts(&split_limit(&s("a,b,c,d"), &s(","), Some(0))).len(), 0);
        assert_eq!(parts(&split_limit(&s("a,b,c,d"), &s(","), Some(10))), ["a", "b", "c", "d"]);
        assert_eq!(parts(&split_limit(&s("a,b,c,d"), &s(","), None)), ["a", "b", "c", "d"]);
        // split() delegates with no limit
        assert_eq!(parts(&split(&s("one two"), &s(" "))), ["one", "two"]);
    }

    #[test]
    fn case_and_prefix_suffix() {
        assert_same!(to_upper_case(&s("aB")), s("AB"));
        assert_same!(to_lower_case(&s("aB")), s("ab"));
        assert_same!(starts_with(&s("/api"), &s("/api"), None), Value::Bool(true));
        assert_same!(ends_with(&s("x.js"), &s(".js"), None), Value::Bool(true));
        assert_same!(starts_with(&n(1.0), &s(""), None), Value::Bool(false));
        // 2nd-arg: position / endPosition.
        assert_same!(starts_with(&s("abc"), &s("bc"), Some(&n(1.0))), Value::Bool(true));
        assert_same!(ends_with(&s("abc"), &s("ab"), Some(&n(2.0))), Value::Bool(true));
    }

    #[test]
    fn replace_family() {
        assert_same!(replace(&s("aa"), &s("a"), &s("b")), s("ba"));
        assert_same!(replace_all(&s("aa"), &s("a"), &s("b")), s("bb"));
        assert_same!(replace(&n(1.0), &s("a"), &s("b")), Value::Null);
    }

    #[test]
    fn char_at_code() {
        assert_same!(char_at(&s("ab"), &n(0.0)), s("a"));
        assert_same!(char_at(&s("ab"), &n(99.0)), s(""));
        if let Value::Number(x) = char_code_at(&s("A"), &n(0.0)) {
            assert_eq!(x, 65.0);
        } else {
            panic!();
        }
        assert!(matches!(char_code_at(&s("x"), &n(9.0)), Value::Number(x) if x.is_nan()));
    }

    #[test]
    fn repeat_pad() {
        assert_same!(repeat(&s("ab"), &n(2.0)), s("abab"));
        assert_same!(repeat(&s("x"), &n(0.0)), s(""));
        assert_same!(pad_start(&s("5"), &n(3.0), &s("0")), s("005"));
        assert_same!(pad_end(&s("hi"), &n(5.0), &s("!")), s("hi!!!"));
        assert_same!(pad_start(&s("hello"), &n(3.0), &Value::Null), s("hello"));
    }

    #[test]
    fn last_index_of_basic() {
        assert_same!(
            last_index_of(&s("abcabc"), &s("a"), &n(f64::INFINITY)),
            n(3.0)
        );
        assert_same!(last_index_of(&s("abcabc"), &s("a"), &n(2.0)), n(0.0));
        assert_same!(last_index_of(&s("hello"), &s("l"), &n(3.0)), n(3.0));
        assert_same!(last_index_of(&s("hello"), &s("l"), &n(1.0)), n(-1.0));
    }

    #[test]
    fn last_index_of_omit_and_null() {
        assert_same!(last_index_of(&s("aba"), &s("a"), &n(f64::INFINITY)), n(2.0));
        assert_same!(last_index_of(&s("aba"), &s("a"), &Value::Null), n(0.0));
    }

    #[test]
    fn last_index_of_empty_needle() {
        assert_same!(last_index_of(&s("abc"), &s(""), &n(2.0)), n(2.0));
    }

    #[test]
    fn last_index_of_nan_position() {
        assert_same!(last_index_of(&s("aba"), &s("a"), &n(f64::NAN)), n(0.0));
    }

    #[test]
    fn last_index_of_unicode() {
        assert_same!(
            last_index_of(&s("😀a😀"), &s("a"), &n(f64::INFINITY)),
            n(1.0)
        );
        assert_same!(
            last_index_of(&s("😀a😀"), &s("😀"), &n(f64::INFINITY)),
            n(2.0)
        );
    }

    #[test]
    fn last_index_of_non_string() {
        assert_same!(last_index_of(&n(1.0), &s("a"), &n(0.0)), n(-1.0));
    }

    #[test]
    fn escape_html_basic() {
        assert_same!(escape_html(&s("plain text")), s("plain text"));
        assert_same!(
            escape_html(&s("<script>alert(\"xss\")</script>")),
            s("&lt;script&gt;alert(&quot;xss&quot;)&lt;/script&gt;")
        );
        assert_same!(escape_html(&s("tom & jerry")), s("tom &amp; jerry"));
        assert_same!(escape_html(&s("it's")), s("it&#39;s"));
        assert_same!(
            escape_html(&s("<script>alert('x' & \"y\");</script>")),
            s("&lt;script&gt;alert(&#39;x&#39; &amp; &quot;y&quot;);&lt;/script&gt;")
        );
    }

    #[test]
    fn escape_html_unicode_preserved() {
        // Astral symbols / non-ASCII must round-trip unchanged.
        assert_same!(escape_html(&s("フレーム")), s("フレーム"));
        assert_same!(escape_html(&s("🎉 & 💥")), s("🎉 &amp; 💥"));
    }
}
