//! Span conversion from OXC offsets to Tish (line, col).

use oxc::span::GetSpan;
use tish_ast::Span;

/// Convert OXC span (byte start/end) to Tish Span (line, col).
/// Uses source text to compute line/column from offsets.
pub fn oxc_span_to_tish<T: GetSpan>(source: &str, oxc_span: &T) -> Span {
    let s = oxc_span.span();
    let start = offset_to_line_col(source, s.start);
    let end = offset_to_line_col(source, s.end);
    Span { start, end }
}

/// Compute (line, col) for byte offset. Lines and columns are 0-based.
fn offset_to_line_col(source: &str, offset: u32) -> (usize, usize) {
    let offset = offset as usize;
    if offset >= source.len() {
        let lines = source.lines().count();
        let last_line_len = source.lines().last().map(str::len).unwrap_or(0);
        return (lines.saturating_sub(1), last_line_len);
    }
    let prefix = &source[..offset];
    let line = prefix.lines().count().saturating_sub(1);
    let col = prefix.lines().last().map(str::len).unwrap_or(0);
    (line, col)
}

/// Stub span when source is not available (e.g. synthetic nodes).
pub fn stub_span() -> Span {
    Span {
        start: (0, 0),
        end: (0, 0),
    }
}
