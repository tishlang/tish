//! Map LSP (UTF-16) positions to lexer [`Span`] corners (1-based line, 1-based **character** column).

use tishlang_ast::Span;

/// Byte offset in `source` for LSP `Position` (0-based line, UTF-16 code unit column).
pub fn lsp_position_to_byte_offset(source: &str, line: u32, character: u32) -> Option<usize> {
    let mut cur_line: u32 = 0;
    let mut col_utf16: u32 = 0;
    for (i, ch) in source.char_indices() {
        if cur_line == line && col_utf16 == character {
            return Some(i);
        }
        if ch == '\n' {
            if cur_line == line {
                return Some(i);
            }
            cur_line += 1;
            col_utf16 = 0;
        } else if cur_line == line {
            col_utf16 = col_utf16.saturating_add(ch.len_utf16() as u32);
        }
    }
    if cur_line == line && col_utf16 == character {
        Some(source.len())
    } else {
        None
    }
}

fn line_byte_start(source: &str, line1: usize) -> Option<usize> {
    if line1 == 0 {
        return None;
    }
    if line1 == 1 {
        return Some(0);
    }
    let mut line = 1usize;
    let mut offset = 0usize;
    for ch in source.chars() {
        offset += ch.len_utf8();
        if ch == '\n' {
            line += 1;
            if line == line1 {
                return Some(offset);
            }
        }
    }
    None
}

/// Byte offset for lexer (1-based line, 1-based char column) — first character of that cell.
pub fn lex_corner_to_byte_offset(source: &str, line1: usize, col1: usize) -> Option<usize> {
    let line_str = source.lines().nth(line1.checked_sub(1)?)?;
    let base = line_byte_start(source, line1)?;
    let mut pos = base;
    let mut c = 1usize;
    for ch in line_str.chars() {
        if c == col1 {
            return Some(pos);
        }
        c += 1;
        pos += ch.len_utf8();
    }
    if c == col1 {
        Some(pos)
    } else {
        None
    }
}

/// Half-open byte range `[start, end)` for lexer span corners.
pub fn lex_span_byte_range(source: &str, span: &Span) -> Option<(usize, usize)> {
    let start = lex_corner_to_byte_offset(source, span.start.0, span.start.1)?;
    let end = lex_corner_to_byte_offset(source, span.end.0, span.end.1).unwrap_or(start);
    let end = end.max(start);
    Some((start, end))
}

/// LSP `(line, character)` for the lexer corner `span.start` (first character of the span).
pub fn lsp_position_for_span_start(source: &str, span: &Span) -> Option<(u32, u32)> {
    let b = lex_corner_to_byte_offset(source, span.start.0, span.start.1)?;
    byte_offset_to_lsp(source, b)
}

pub fn byte_offset_to_lsp(source: &str, byte: usize) -> Option<(u32, u32)> {
    let mut line = 0u32;
    let mut utf16 = 0u32;
    for (i, ch) in source.char_indices() {
        if i == byte {
            return Some((line, utf16));
        }
        if ch == '\n' {
            line += 1;
            utf16 = 0;
        } else {
            utf16 += ch.len_utf16() as u32;
        }
    }
    if byte == source.len() {
        Some((line, utf16))
    } else {
        None
    }
}

/// True if LSP position lies inside `span` (half-open in byte space).
pub fn span_contains_lsp_position(source: &str, span: &Span, lsp_line: u32, lsp_character: u32) -> bool {
    let Some(b) = lsp_position_to_byte_offset(source, lsp_line, lsp_character) else {
        return false;
    };
    let Some((lo, hi)) = lex_span_byte_range(source, span) else {
        return false;
    };
    b >= lo && b < hi
}

/// LSP start (inclusive) and end (exclusive) positions for a lexer span.
pub fn span_to_lsp_range_exclusive(source: &str, span: &Span) -> Option<((u32, u32), (u32, u32))> {
    let (lo, hi) = lex_span_byte_range(source, span)?;
    Some((byte_offset_to_lsp(source, lo)?, byte_offset_to_lsp(source, hi)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lsp_first_char() {
        let s = "let x = 1\n";
        assert_eq!(lsp_position_to_byte_offset(s, 0, 0), Some(0));
        assert_eq!(lsp_position_to_byte_offset(s, 0, 4), Some(4));
    }
}
