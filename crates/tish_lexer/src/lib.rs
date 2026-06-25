//! Tish lexer with indent normalization and tab/space handling.
//!
//! Normalizes tabs and spaces to a single indent level so both styles work.
//! Emits virtual Indent/Dedent tokens for optional-brace blocks.

mod token;

pub use token::{Span, Token, TokenKind};

use std::collections::VecDeque;
use std::iter::Peekable;
use std::str::Chars;

const INDENT_WIDTH: usize = 2;
const TAB_AS_LEVELS: usize = 1;

/// One JSX element on the stack: tracks whether we are still in its opening tag (`<Tag ...`)
/// and how many `{` are open inside that element's **attribute values** (embedded JS).
/// This lets `>` be a comparison operator inside `{...}` while still closing `<span>` when
/// `attr_value_braces == 0` for the innermost element (React-like).
#[derive(Debug, Clone)]
struct JsxEl {
    in_opener: bool,
    attr_value_braces: i32,
}

/// Lexer configuration.
#[derive(Debug, Clone, Copy, Default)]
pub struct LexerOptions {
    /// When true, suppress the virtual `Indent`/`Dedent` tokens so blocks are delimited
    /// **only** by braces. Indentation is treated as ordinary whitespace, so off-side
    /// (brace-less) blocks no longer form. Useful for debugging how nested blocks
    /// transpile — see the `TISH_IGNORE_INDENT` environment variable for a global toggle.
    pub ignore_indent: bool,
}

impl LexerOptions {
    /// Build options from the environment. `TISH_IGNORE_INDENT=1` (or `true`/`yes`) sets
    /// `ignore_indent`, so every parse path (run/build/dump-ast/fmt/lint/lsp) honors it
    /// without threading a flag through the whole pipeline.
    pub fn from_env() -> Self {
        Self {
            ignore_indent: env_truthy(std::env::var_os("TISH_IGNORE_INDENT")),
        }
    }
}

/// Interpret an environment-variable value as a boolean flag: `1`, `true`, or `yes`
/// (exact, case-sensitive) enable it; anything else — including unset — leaves it off.
/// Split out from the `std::env` read so the rule is unit-testable without mutating
/// process-global state (which `Lexer::new` reads, so env-mutating tests would race).
fn env_truthy(value: Option<std::ffi::OsString>) -> bool {
    value
        .map(|v| v == "1" || v == "true" || v == "yes")
        .unwrap_or(false)
}

#[derive(Debug, Clone)]
pub struct Lexer<'a> {
    chars: Peekable<Chars<'a>>,
    pos: usize,
    line: usize,
    col: usize,
    indent_stack: Vec<usize>,
    at_line_start: bool,
    pending_dedents: VecDeque<Token>,
    template_brace_stack: Vec<usize>,
    jsx_after_gt: bool,
    jsx_in_opening_tag: bool,
    jsx_saw_slash_before_gt: bool,
    jsx_stack: Vec<JsxEl>,
    jsx_depth: i32,
    jsx_child_brace_depth: i32,
    jsx_in_closing_tag: bool,
    ignore_indent: bool,
    /// Kind of the last emitted significant token, for `<` disambiguation: after a *value* position
    /// (ident, `)`, `]`, literal) a `<` is a comparison / generic-args opener (`Lt`), never a JSX tag.
    last_significant_kind: Option<TokenKind>,
}

impl<'a> Lexer<'a> {
    /// Create a lexer, reading options from the environment (e.g. `TISH_IGNORE_INDENT`).
    pub fn new(source: &'a str) -> Self {
        Self::with_options(source, LexerOptions::from_env())
    }

    /// Create a lexer with explicit options, bypassing the environment.
    pub fn with_options(source: &'a str, options: LexerOptions) -> Self {
        Self {
            chars: source.chars().peekable(),
            pos: 0,
            line: 1,
            col: 1,
            indent_stack: vec![0],
            at_line_start: true,
            pending_dedents: VecDeque::new(),
            template_brace_stack: Vec::new(),
            jsx_after_gt: false,
            jsx_in_opening_tag: false,
            jsx_saw_slash_before_gt: false,
            jsx_stack: Vec::new(),
            jsx_depth: 0,
            jsx_child_brace_depth: 0,
            jsx_in_closing_tag: false,
            ignore_indent: options.ignore_indent,
            last_significant_kind: None,
        }
    }

    /// True when the previous significant token ends a value, so a following `<` is `Lt`
    /// (comparison / generic args), not the start of a JSX element.
    fn last_is_value(&self) -> bool {
        matches!(
            self.last_significant_kind,
            Some(
                TokenKind::Ident
                    | TokenKind::RParen
                    | TokenKind::RBracket
                    | TokenKind::Number
                    | TokenKind::String
                    | TokenKind::True
                    | TokenKind::False
                    | TokenKind::Null
            )
        )
    }

    /// #299 — true when the previous significant token ends a value, so a following `/` is DIVISION,
    /// not the start of a regex literal. Superset of `last_is_value` (adds postfix `++`/`--` and
    /// template-end tokens, which also end a value). When false, `/` begins a regex.
    fn prev_ends_value(&self) -> bool {
        self.last_is_value()
            || matches!(
                self.last_significant_kind,
                Some(
                    TokenKind::PlusPlus
                        | TokenKind::MinusMinus
                        | TokenKind::TemplateNoSub
                        | TokenKind::TemplateTail
                )
            )
    }

    /// #299 — scan a regex literal body. The opening `/` is already consumed. Reads the pattern
    /// VERBATIM (no escape processing — `\d` must stay `\d`), honoring `\`-escapes (`\/` is literal)
    /// and `[...]` character classes (a `/` inside a class is literal), until the closing `/`, then
    /// the trailing ascii-alpha flags. A newline inside the body or EOF is an unterminated-regex error.
    fn read_regex(&mut self) -> Result<(String, String), String> {
        let mut pat = String::with_capacity(16);
        let mut in_class = false;
        loop {
            match self.advance() {
                None | Some('\n') => return Err("Unterminated regex literal".to_string()),
                Some('\\') => {
                    pat.push('\\');
                    match self.advance() {
                        None | Some('\n') => {
                            return Err("Unterminated regex literal".to_string())
                        }
                        Some(c) => pat.push(c),
                    }
                }
                Some('[') => {
                    in_class = true;
                    pat.push('[');
                }
                Some(']') => {
                    in_class = false;
                    pat.push(']');
                }
                Some('/') if !in_class => break,
                Some(c) => pat.push(c),
            }
        }
        let mut flags = String::new();
        while let Some(c) = self.peek() {
            if c.is_ascii_alphabetic() {
                flags.push(c);
                self.advance();
            } else {
                break;
            }
        }
        Ok((pat, flags))
    }

    #[inline]
    fn jsx_sync_in_opening_tag(&mut self) {
        self.jsx_in_opening_tag = self.jsx_stack.last().map(|e| e.in_opener).unwrap_or(false);
    }

    fn read_jsx_text(&mut self, start: (usize, usize)) -> Result<Option<Token>, String> {
        let mut s = String::new();
        loop {
            match self.peek() {
                None | Some('{') | Some('<') => break,
                Some(c) => {
                    self.advance();
                    s.push(c);
                }
            }
        }
        if s.is_empty() {
            Ok(None)
        } else {
            let end = self.span_start();
            Ok(Some(Token {
                kind: TokenKind::JsxText,
                span: Span { start, end },
                literal: Some(s.into()),
            }))
        }
    }

    fn peek(&mut self) -> Option<char> {
        self.chars.peek().copied()
    }

    fn advance(&mut self) -> Option<char> {
        let c = self.chars.next()?;
        self.pos += c.len_utf8();
        if c == '\n' {
            self.line += 1;
            self.col = 1;
            self.at_line_start = true;
        } else {
            self.col += 1;
        }
        Some(c)
    }

    fn span_start(&self) -> (usize, usize) {
        (self.line, self.col)
    }

    fn read_indent_level(&mut self) -> usize {
        let mut level = 0;
        loop {
            match self.peek() {
                Some(' ') => {
                    self.advance();
                    level += 1;
                }
                Some('\t') => {
                    self.advance();
                    level += TAB_AS_LEVELS;
                }
                _ => break,
            }
        }
        level.div_ceil(INDENT_WIDTH)
    }

    fn skip_whitespace(&mut self) {
        while let Some(c) = self.peek() {
            if c == ' ' || c == '\t' || c == '\r' {
                self.advance();
            } else if c == '\n' {
                self.advance();
                self.at_line_start = true;
            } else {
                break;
            }
        }
    }

    fn skip_line_comment(&mut self) {
        while let Some(c) = self.advance() {
            if c == '\n' {
                break;
            }
        }
    }

    fn skip_block_comment(&mut self) -> Result<(), String> {
        let mut depth = 1;
        while depth > 0 {
            match self.advance() {
                Some('*') if self.peek() == Some('/') => {
                    self.advance();
                    depth -= 1;
                }
                Some('/') if self.peek() == Some('*') => {
                    self.advance();
                    depth += 1;
                }
                None => return Err("Unterminated block comment".to_string()),
                _ => {}
            }
        }
        Ok(())
    }

    fn read_number(&mut self, first: char) -> String {
        // Radix-prefixed integer literals: `0x`/`0X` (hex), `0o`/`0O` (octal), `0b`/`0B`
        // (binary), with optional `_` digit separators. JS semantics — a non-negative
        // integer. Convert to a decimal string here so every downstream consumer (the
        // parser's `parse::<f64>()`, the formatter, …) sees a plain number, unchanged.
        if first == '0' {
            if let Some(radix) = self.radix_prefix() {
                self.advance(); // consume the x/o/b marker
                let mut digits = String::with_capacity(16);
                while let Some(c) = self.peek() {
                    if c == '_' {
                        self.advance(); // digit separator
                    } else if c.is_digit(radix) {
                        digits.push(c);
                        self.advance();
                    } else {
                        break;
                    }
                }
                return Self::radix_digits_to_decimal(&digits, radix);
            }
        }

        let mut s = String::with_capacity(16);
        s.push(first);
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() || c == '.' {
                s.push(c);
                self.advance();
            } else if c == '_' && Self::ends_with_digit(&s) && self.underscore_between_digits() {
                self.advance(); // numeric separator (`15_000`) — drop it, JS-style
            } else if (c == 'e' || c == 'E') && self.exponent_follows() {
                // Scientific notation: `e`/`E` then optional sign then digits.
                // Guarded by lookahead so `3em` lexes as `3` + `em`, not a bad number.
                s.push(c);
                self.advance(); // consume e/E
                if matches!(self.peek(), Some('+') | Some('-')) {
                    s.push(self.peek().unwrap());
                    self.advance();
                }
                while let Some(d) = self.peek() {
                    if d.is_ascii_digit() {
                        s.push(d);
                        self.advance();
                    } else if d == '_'
                        && Self::ends_with_digit(&s)
                        && self.underscore_between_digits()
                    {
                        self.advance(); // numeric separator inside the exponent (`1e1_0`)
                    } else {
                        break;
                    }
                }
                break; // the exponent terminates the numeric literal
            } else {
                break;
            }
        }
        s
    }

    /// True iff the literal accumulated so far ends in a decimal digit — used to reject a
    /// `_` separator that isn't preceded by a digit (e.g. leading `_5` or post-`.` `1._5`).
    fn ends_with_digit(s: &str) -> bool {
        s.chars().last().is_some_and(|c| c.is_ascii_digit())
    }

    /// With `peek()` positioned at a `_`, look ahead (without consuming) to confirm the
    /// next character is a decimal digit, i.e. the `_` sits between two digits and is a
    /// valid JS numeric separator (rejects trailing `5_` and doubled `1__0`).
    fn underscore_between_digits(&self) -> bool {
        let mut la = self.chars.clone();
        la.next(); // skip the `_` currently under peek()
        la.next().is_some_and(|c| c.is_ascii_digit())
    }

    /// With the current peek positioned at an `e`/`E`, decide (without consuming)
    /// whether a valid exponent `[+-]?\d` follows. `Chars` is `Clone`, so we look
    /// ahead on a throwaway clone of the iterator.
    fn exponent_follows(&self) -> bool {
        let mut la = self.chars.clone();
        la.next(); // skip the e/E currently under peek()
        match la.next() {
            Some(d) if d.is_ascii_digit() => true,
            Some('+') | Some('-') => la.next().is_some_and(|d| d.is_ascii_digit()),
            _ => false,
        }
    }

    /// With a leading `0` already consumed and `peek()` at the radix marker, return the
    /// radix (16 / 8 / 2) iff this is a valid `0x` / `0o` / `0b` prefix followed by at
    /// least one valid digit. Returns `None` otherwise, so `0`, `0.5`, `0e3`, `0xZ`, and
    /// `0x_1` all stay on the decimal path. Looks ahead on a clone of the `Chars` iterator
    /// (`Chars: Clone`) without consuming.
    fn radix_prefix(&self) -> Option<u32> {
        let mut la = self.chars.clone();
        let radix = match la.next()? {
            'x' | 'X' => 16,
            'o' | 'O' => 8,
            'b' | 'B' => 2,
            _ => return None,
        };
        match la.next() {
            Some(c) if c.is_digit(radix) => Some(radix),
            _ => None,
        }
    }

    /// Convert the (separator-free) digits of a radix-prefixed literal to the decimal
    /// string the `Number` token carries. `u128` is exact for ≤128-bit literals — far
    /// beyond any real input; the `f64` fallback only triggers for absurdly long ones and
    /// loses precision past 2^53, exactly as JS's conversion to a double would.
    fn radix_digits_to_decimal(digits: &str, radix: u32) -> String {
        if let Ok(v) = u128::from_str_radix(digits, radix) {
            return v.to_string();
        }
        let mut v = 0.0_f64;
        for c in digits.chars() {
            v = v * radix as f64 + c.to_digit(radix).unwrap_or(0) as f64;
        }
        format!("{v}")
    }

    /// Handle escape sequence, returning the unescaped character.
    /// `extra_allowed` contains additional characters that can be escaped in this context.
    fn handle_escape(&mut self, extra_allowed: &[char]) -> Result<char, String> {
        let escaped = self.advance().ok_or("Unterminated escape")?;
        match escaped {
            'n' => Ok('\n'),
            'r' => Ok('\r'),
            't' => Ok('\t'),
            'b' => Ok('\u{0008}'),
            'f' => Ok('\u{000C}'),
            'v' => Ok('\u{000B}'),
            '0' => Ok('\0'),
            '\\' => Ok('\\'),
            // `\xNN` — exactly two hex digits → code point 0x00..=0xFF (JS/TS).
            'x' => {
                let cp = self.read_hex_digits(2)?;
                char::from_u32(cp).ok_or_else(|| format!("Invalid \\x escape: \\x{:02X}", cp))
            }
            // `\uNNNN` (exactly four hex digits) or `\u{N..}` (1-6 hex digits, ES6).
            'u' => {
                let cp = if self.peek() == Some('{') {
                    self.advance(); // consume '{'
                    let cp = self.read_hex_until_brace()?;
                    match self.advance() {
                        Some('}') => cp,
                        _ => return Err("Unterminated \\u{...} escape (expected '}')".to_string()),
                    }
                } else {
                    self.read_hex_digits(4)?
                };
                // Lone surrogates (0xD800..=0xDFFF) are valid UTF-16 code units in JS but
                // not Unicode scalar values; tish strings are UTF-8, so reject them.
                char::from_u32(cp)
                    .ok_or_else(|| format!("Invalid \\u escape: code point U+{:04X}", cp))
            }
            c if extra_allowed.contains(&c) => Ok(c),
            _ => Err(format!("Unknown escape: \\{}", escaped)),
        }
    }

    /// Read exactly `n` hex digits and return the parsed code point.
    fn read_hex_digits(&mut self, n: usize) -> Result<u32, String> {
        let mut value: u32 = 0;
        for _ in 0..n {
            let c = self.advance().ok_or("Unterminated hex escape")?;
            let digit = c
                .to_digit(16)
                .ok_or_else(|| format!("Invalid hex digit in escape: '{}'", c))?;
            value = value * 16 + digit;
        }
        Ok(value)
    }

    /// Read 1-6 hex digits for a `\u{...}` escape (stops at `}`); validates the count
    /// and that the value is within the Unicode range.
    fn read_hex_until_brace(&mut self) -> Result<u32, String> {
        let mut value: u32 = 0;
        let mut count = 0;
        while let Some(c) = self.peek() {
            let Some(digit) = c.to_digit(16) else { break };
            self.advance();
            value = value * 16 + digit;
            count += 1;
            if count > 6 || value > 0x10_FFFF {
                return Err("Invalid \\u{...} escape: code point out of range".to_string());
            }
        }
        if count == 0 {
            return Err("Empty \\u{} escape (expected hex digits)".to_string());
        }
        Ok(value)
    }

    fn read_string(&mut self, quote: char) -> Result<String, String> {
        let mut s = String::with_capacity(32);
        let extra = if quote == '"' {
            &['"', '\''][..]
        } else {
            &['\'', '"'][..]
        };
        loop {
            match self.advance() {
                None => return Err("Unterminated string".to_string()),
                Some(c) if c == quote => break,
                Some('\\') => s.push(self.handle_escape(extra)?),
                Some(c) => s.push(c),
            }
        }
        Ok(s)
    }

    fn read_ident_or_keyword(&mut self, first: char) -> String {
        let mut s = String::with_capacity(16);
        s.push(first);
        while let Some(c) = self.peek() {
            if c.is_ascii_alphanumeric() || c == '_' {
                s.push(c);
                self.advance();
            } else {
                break;
            }
        }
        s
    }

    /// Read a template literal. If `is_continuation` is true, we're continuing after a `}`.
    fn read_template(
        &mut self,
        start: (usize, usize),
        is_continuation: bool,
    ) -> Result<Option<Token>, String> {
        let mut s = String::with_capacity(if is_continuation { 32 } else { 64 });
        let extra = &['`', '$', '{'][..];

        loop {
            match self.advance() {
                None => return Err("Unterminated template literal".to_string()),
                Some('`') => {
                    let end = self.span_start();
                    let kind = if is_continuation {
                        TokenKind::TemplateTail
                    } else {
                        TokenKind::TemplateNoSub
                    };
                    return Ok(Some(Token {
                        kind,
                        span: Span { start, end },
                        literal: Some(s.into()),
                    }));
                }
                Some('$') if self.peek() == Some('{') => {
                    self.advance();
                    self.template_brace_stack.push(1);
                    let end = self.span_start();
                    let kind = if is_continuation {
                        TokenKind::TemplateMiddle
                    } else {
                        TokenKind::TemplateHead
                    };
                    return Ok(Some(Token {
                        kind,
                        span: Span { start, end },
                        literal: Some(s.into()),
                    }));
                }
                Some('\\') => s.push(self.handle_escape(extra)?),
                Some(c) => s.push(c),
            }
        }
    }

    fn emit_indent_or_dedent(&mut self, level: usize) -> Option<Token> {
        let top = *self.indent_stack.last().unwrap();
        let start = self.span_start();

        if level > top {
            self.indent_stack.push(level);
            Some(Token {
                kind: TokenKind::Indent,
                span: Span { start, end: start },
                literal: None,
            })
        } else if level < top {
            while self.indent_stack.len() > 1 && *self.indent_stack.last().unwrap() > level {
                self.indent_stack.pop();
                self.pending_dedents.push_back(Token {
                    kind: TokenKind::Dedent,
                    span: Span { start, end: start },
                    literal: None,
                });
            }
            if *self.indent_stack.last().unwrap_or(&0) != level {
                self.indent_stack.push(level);
            }
            self.pending_dedents.pop_front()
        } else {
            None
        }
    }

    pub fn next_token(&mut self) -> Result<Option<Token>, String> {
        let tok = self.next_token_inner()?;
        if let Some(t) = &tok {
            self.last_significant_kind = Some(t.kind);
        }
        Ok(tok)
    }

    fn next_token_inner(&mut self) -> Result<Option<Token>, String> {
        if let Some(tok) = self.pending_dedents.pop_front() {
            return Ok(Some(tok));
        }

        if self.jsx_after_gt {
            self.jsx_after_gt = false;
            if !matches!(self.peek(), Some('{') | Some('<') | None) {
                let start = self.span_start();
                if let Some(tok) = self.read_jsx_text(start)? {
                    return Ok(Some(tok));
                }
            }
        }

        if self.at_line_start {
            self.at_line_start = false;
            // Always consume the leading whitespace; only *emit* Indent/Dedent when indentation
            // is significant. With `ignore_indent`, the level is discarded so the indent stack
            // stays at `[0]` and no virtual tokens are produced (brace-only blocks).
            let level = self.read_indent_level();
            if !self.ignore_indent
                && (level > 0 || self.peek().map(|c| c != '\n').unwrap_or(false))
            {
                if let Some(tok) = self.emit_indent_or_dedent(level) {
                    return Ok(Some(tok));
                }
            }
        }

        self.skip_whitespace();
        if self.at_line_start {
            return self.next_token();
        }

        let start = self.span_start();
        let c = match self.advance() {
            Some(c) => c,
            None => {
                if let Some(tok) = self.pending_dedents.pop_front() {
                    return Ok(Some(tok));
                }
                if self.indent_stack.len() > 1 {
                    self.indent_stack.pop();
                    return Ok(Some(Token {
                        kind: TokenKind::Dedent,
                        span: Span {
                            start: (self.line, self.col),
                            end: (self.line, self.col),
                        },
                        literal: None,
                    }));
                }
                return Ok(None);
            }
        };

        let kind = match c {
            '(' => TokenKind::LParen,
            ')' => TokenKind::RParen,
            '{' => {
                if self.jsx_in_opening_tag {
                    if let Some(top) = self.jsx_stack.last_mut() {
                        top.attr_value_braces += 1;
                    }
                } else if self.jsx_depth > 0 {
                    self.jsx_child_brace_depth += 1;
                }
                if let Some(depth) = self.template_brace_stack.last_mut() {
                    *depth += 1;
                }
                TokenKind::LBrace
            }
            '}' => {
                let mut handled = false;
                if let Some(top) = self.jsx_stack.last() {
                    if top.in_opener && top.attr_value_braces > 0 {
                        if let Some(top) = self.jsx_stack.last_mut() {
                            top.attr_value_braces -= 1;
                        }
                        handled = true;
                    }
                }
                if !handled && self.jsx_child_brace_depth > 0 {
                    self.jsx_child_brace_depth -= 1;
                    if self.jsx_child_brace_depth == 0 {
                        self.jsx_after_gt = true;
                    }
                }
                if let Some(depth) = self.template_brace_stack.last_mut() {
                    *depth -= 1;
                    if *depth == 0 {
                        self.template_brace_stack.pop();
                        return self.read_template(start, true);
                    }
                }
                TokenKind::RBrace
            }
            '[' => TokenKind::LBracket,
            ']' => TokenKind::RBracket,
            ';' => TokenKind::Semicolon,
            ',' => TokenKind::Comma,
            '.' => {
                if self.peek() == Some('?') {
                    self.advance();
                    TokenKind::OptionalChain
                } else if self.peek() == Some('.') {
                    self.advance();
                    if self.peek() == Some('.') {
                        self.advance();
                        TokenKind::Spread
                    } else {
                        return Err("Unexpected .. (use ... for rest params)".to_string());
                    }
                } else {
                    TokenKind::Dot
                }
            }
            '=' => {
                if self.peek() == Some('=') {
                    self.advance();
                    if self.peek() == Some('=') {
                        self.advance();
                        TokenKind::StrictEq
                    } else {
                        TokenKind::Eq
                    }
                } else if self.peek() == Some('>') {
                    self.advance();
                    TokenKind::Arrow
                } else {
                    TokenKind::Assign
                }
            }
            '!' => {
                if self.peek() == Some('=') {
                    self.advance();
                    if self.peek() == Some('=') {
                        self.advance();
                        TokenKind::StrictNe
                    } else {
                        TokenKind::Ne
                    }
                } else {
                    TokenKind::Not
                }
            }
            '<' => {
                if self.peek() == Some('=') {
                    self.advance();
                    TokenKind::Le
                } else if self.peek() == Some('<') {
                    self.advance();
                    TokenKind::Shl
                } else if self.peek() == Some('/') {
                    self.jsx_in_closing_tag = true;
                    TokenKind::Lt
                } else if (self.peek() == Some('>')
                    || self
                        .peek()
                        .map(|c| c.is_ascii_alphabetic() || c == '_')
                        .unwrap_or(false))
                    && !self.last_is_value()
                {
                    // JSX open tag — only in expression position. After a value (`ident<`, `)<`,
                    // `]<`, literal) this is `Lt`: a comparison or generic-args opener.
                    self.jsx_depth += 1;
                    self.jsx_stack.push(JsxEl {
                        in_opener: true,
                        attr_value_braces: 0,
                    });
                    self.jsx_in_opening_tag = true;
                    TokenKind::Lt
                } else {
                    TokenKind::Lt
                }
            }
            '>' => {
                if self.peek() == Some('=') {
                    self.advance();
                    TokenKind::Ge
                } else if self.peek() == Some('>') {
                    self.advance();
                    if self.peek() == Some('>') {
                        self.advance();
                        TokenKind::UShr // `>>>`
                    } else {
                        TokenKind::Shr
                    }
                } else {
                    if self.jsx_in_closing_tag
                        || (self.jsx_in_opening_tag && self.jsx_saw_slash_before_gt)
                    {
                        self.jsx_depth = (self.jsx_depth - 1).max(0);
                        self.jsx_stack.pop();
                        self.jsx_sync_in_opening_tag();
                        // A child element just closed (`</span>` or `<br/>`). If a parent element
                        // is still open and past its opening tag, we're back in that parent's
                        // children region, so the following run is JSX text — re-enter text mode.
                        // Without this, trailing text after a child element ("… as JSON") is lexed
                        // as code and a bare keyword (`as`, `in`, `if`, …) breaks the parse (#108).
                        //
                        // Guard on `jsx_child_brace_depth == 0`: if the closed element lived inside a
                        // `{…}` expression container (e.g. `<div>{items.map(x => <span/>)}</div>`),
                        // we're still in that expression, not the parent's text children — entering
                        // text mode there would swallow the following `)`/`,` as JsxText.
                        if self.jsx_child_brace_depth == 0
                            && self.jsx_stack.last().map(|e| !e.in_opener).unwrap_or(false)
                        {
                            self.jsx_after_gt = true;
                        }
                    } else if let Some(top) = self.jsx_stack.last_mut() {
                        if top.in_opener && top.attr_value_braces > 0 {
                            // `>` is a comparison (or shift) token inside `{ ... }`, not end of opening tag.
                        } else if top.in_opener && !self.jsx_saw_slash_before_gt {
                            top.in_opener = false;
                            self.jsx_after_gt = true;
                            self.jsx_sync_in_opening_tag();
                        }
                    }
                    self.jsx_in_closing_tag = false;
                    self.jsx_saw_slash_before_gt = false;
                    TokenKind::Gt
                }
            }
            '^' => TokenKind::BitXor,
            '~' => TokenKind::BitNot,
            '+' => {
                if self.peek() == Some('+') {
                    self.advance();
                    TokenKind::PlusPlus
                } else if self.peek() == Some('=') {
                    self.advance();
                    TokenKind::PlusAssign
                } else {
                    TokenKind::Plus
                }
            }
            '-' => {
                if self.peek() == Some('-') {
                    self.advance();
                    TokenKind::MinusMinus
                } else if self.peek() == Some('=') {
                    self.advance();
                    TokenKind::MinusAssign
                } else {
                    TokenKind::Minus
                }
            }
            '*' => {
                if self.peek() == Some('*') {
                    self.advance();
                    TokenKind::StarStar
                } else if self.peek() == Some('=') {
                    self.advance();
                    TokenKind::StarAssign
                } else {
                    TokenKind::Star
                }
            }
            '/' => {
                if self.peek() == Some('/') {
                    self.advance();
                    self.skip_line_comment();
                    // `skip_line_comment` consumes the newline via `advance()`, which sets
                    // `at_line_start` before we would normally run `skip_whitespace()`. Without
                    // stripping the next line's leading spaces here, `read_indent_level` would see
                    // physical indentation and emit a spurious `Indent` (breaks e.g. object
                    // literals with trailing `//` comments). Newlines handled in `skip_whitespace`
                    // eat those spaces before the indent pass; match that behavior.
                    self.skip_whitespace();
                    return self.next_token();
                } else if self.peek() == Some('*') {
                    self.advance();
                    self.skip_block_comment()?;
                    return self.next_token();
                } else if !self.prev_ends_value() && !self.jsx_in_opening_tag {
                    // #299: regex literal — the previous token does not end a value, so `/` starts a
                    // regex (`let re = /\d+/g`, `f(/x/)`, `return /x/`), not division. `//` and `/*`
                    // (handled above) stay comments even here. Desugared to `new RegExp(...)` by the parser.
                    let (pat, flags) = self.read_regex()?;
                    let end = self.span_start();
                    let lit = format!("{}\u{0}{}", pat, flags);
                    return Ok(Some(Token {
                        kind: TokenKind::Regex,
                        span: Span { start, end },
                        literal: Some(lit.into()),
                    }));
                } else if self.peek() == Some('=') {
                    self.advance();
                    TokenKind::SlashAssign
                } else {
                    if self.jsx_in_opening_tag {
                        self.jsx_saw_slash_before_gt = true;
                    }
                    TokenKind::Slash
                }
            }
            '%' => {
                if self.peek() == Some('=') {
                    self.advance();
                    TokenKind::PercentAssign
                } else {
                    TokenKind::Percent
                }
            }
            '&' => {
                if self.peek() == Some('&') {
                    self.advance();
                    if self.peek() == Some('=') {
                        self.advance();
                        TokenKind::AndAndAssign
                    } else {
                        TokenKind::And
                    }
                } else {
                    TokenKind::BitAnd
                }
            }
            '|' => {
                if self.peek() == Some('|') {
                    self.advance();
                    if self.peek() == Some('=') {
                        self.advance();
                        TokenKind::OrOrAssign
                    } else {
                        TokenKind::Or
                    }
                } else {
                    TokenKind::BitOr
                }
            }
            '?' => {
                if self.peek() == Some('?') {
                    self.advance();
                    if self.peek() == Some('=') {
                        self.advance();
                        TokenKind::NullishAssign
                    } else {
                        TokenKind::NullishCoalesce
                    }
                } else if self.peek() == Some('.') {
                    self.advance();
                    TokenKind::OptionalChain
                } else {
                    TokenKind::Question
                }
            }
            ':' => TokenKind::Colon,
            '"' | '\'' => {
                let s = self.read_string(c)?;
                let end = self.span_start();
                return Ok(Some(Token {
                    kind: TokenKind::String,
                    span: Span { start, end },
                    literal: Some(s.into()),
                }));
            }
            '`' => return self.read_template(start, false),
            '0'..='9' => {
                let num = self.read_number(c);
                let end = self.span_start();
                return Ok(Some(Token {
                    kind: TokenKind::Number,
                    span: Span { start, end },
                    literal: Some(num.into()),
                }));
            }
            'a'..='z' | 'A'..='Z' | '_' => {
                let ident = self.read_ident_or_keyword(c);
                let end = self.span_start();
                let kind = TokenKind::keyword_or_ident(&ident);
                return Ok(Some(Token {
                    kind,
                    span: Span { start, end },
                    // Spelling is useful for keywords too (e.g. object keys, type names like `type`).
                    literal: Some(ident.into()),
                }));
            }
            '\n' => {
                self.at_line_start = true;
                return self.next_token();
            }
            _ => return Err(format!("Unexpected character: {:?}", c)),
        };

        let end = self.span_start();
        Ok(Some(Token {
            kind,
            span: Span { start, end },
            literal: None,
        }))
    }
}

impl<'a> Iterator for Lexer<'a> {
    type Item = Result<Token, String>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.next_token() {
            Ok(Some(t)) => Some(Ok(t)),
            Ok(None) => None,
            Err(e) => Some(Err(e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_string_literal() {
        let tokens: Vec<_> = Lexer::new(r#""H""#).collect();
        let tokens: Result<Vec<_>, _> = tokens.into_iter().collect();
        let tokens = tokens.unwrap();
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].kind, TokenKind::String);
        assert_eq!(tokens[0].literal.as_deref(), Some("H"));
    }

    #[test]
    fn test_print_string() {
        let tokens: Vec<_> = Lexer::new(r#"print("H")"#).collect();
        let tokens: Result<Vec<_>, _> = tokens.into_iter().collect();
        let tokens = tokens.unwrap();
        let string_tok = tokens.iter().find(|t| t.kind == TokenKind::String).unwrap();
        assert_eq!(string_tok.literal.as_deref(), Some("H"));
    }

    #[test]
    fn radix_integer_literals() {
        // Hex / octal / binary prefixes (any case) convert to a decimal `Number` literal,
        // honoring `_` digit separators.
        let cases = [
            ("0xff", "255"),
            ("0xFF", "255"),
            ("0X1a", "26"),
            ("0o17", "15"),
            ("0O7", "7"),
            ("0b1010", "10"),
            ("0B0", "0"),
            ("0xdeadbeef", "3735928559"),
            ("0xFF_FF", "65535"),
            ("0b1111_0000", "240"),
        ];
        for (src, expected) in cases {
            let tokens = Lexer::new(src).collect::<Result<Vec<_>, _>>().unwrap();
            let num = tokens
                .iter()
                .find(|t| t.kind == TokenKind::Number)
                .unwrap_or_else(|| panic!("no Number token for {src}"));
            assert_eq!(num.literal.as_deref(), Some(expected), "for {src}");
        }
    }

    #[test]
    fn decimal_numeric_separators() {
        // `_` between digits is a JS numeric separator: dropped from the literal value.
        // Issue #57.
        let only_number = |src: &str| -> String {
            let tokens = Lexer::new(src).collect::<Result<Vec<_>, _>>().unwrap();
            let nums: Vec<_> = tokens
                .iter()
                .filter(|t| t.kind == TokenKind::Number)
                .collect();
            assert_eq!(nums.len(), 1, "expected exactly one Number token for {src}");
            // No stray identifier should be produced from the separated digits.
            assert!(
                !tokens.iter().any(|t| t.kind == TokenKind::Ident),
                "unexpected Ident token while lexing {src}"
            );
            nums[0].literal.as_deref().unwrap().to_string()
        };
        assert_eq!(only_number("15_000"), "15000");
        assert_eq!(only_number("1_000_000"), "1000000");
        assert_eq!(only_number("3.14_159"), "3.14159");
        assert_eq!(only_number("1e1_0"), "1e10");
    }

    #[test]
    fn non_radix_zero_prefixed_stays_decimal() {
        // A leading zero is NOT legacy octal; an invalid prefix is not a radix literal.
        let num_literal = |src: &str| -> String {
            Lexer::new(src)
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
                .into_iter()
                .find(|t| t.kind == TokenKind::Number)
                .unwrap()
                .literal
                .as_deref()
                .unwrap()
                .to_string()
        };
        assert_eq!(num_literal("07"), "07"); // decimal, not octal
        assert_eq!(num_literal("0"), "0");
        // `0xZ` → the Number token is just `0`, then `xZ` lexes as an identifier.
        let toks = Lexer::new("0xZ").collect::<Result<Vec<_>, _>>().unwrap();
        assert_eq!(toks[0].kind, TokenKind::Number);
        assert_eq!(toks[0].literal.as_deref(), Some("0"));
        assert_eq!(toks[1].kind, TokenKind::Ident);
    }

    #[test]
    fn line_comment_does_not_emit_spurious_indent_before_next_line() {
        let with_comment = "fn f() {\n  return {\n    a: 1, // c\n    b: 2\n  }\n}\n";
        let tokens: Vec<_> = Lexer::new(with_comment)
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert!(
            !tokens.iter().any(|t| t.kind == TokenKind::Indent),
            "unexpected Indent after line comment: {:?}",
            tokens
                .iter()
                .map(|t| format!("{:?}", t.kind))
                .collect::<Vec<_>>()
        );
    }

    /// A leading-indented line is what actually drives the lexer to emit virtual tokens:
    /// `  a()` opens an indent level (Indent) and the dedented `b()` closes it (Dedent).
    const INDENTED_SRC: &str = "  a()\nb()\n";

    #[test]
    fn default_options_still_emit_indent_and_dedent() {
        let tokens: Vec<_> = Lexer::with_options(INDENTED_SRC, LexerOptions::default())
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert!(
            tokens.iter().any(|t| t.kind == TokenKind::Indent),
            "expected an Indent token in the default (indentation-significant) mode"
        );
        assert!(
            tokens.iter().any(|t| t.kind == TokenKind::Dedent),
            "expected a Dedent token in the default (indentation-significant) mode"
        );
    }

    #[test]
    fn ignore_indent_emits_no_virtual_tokens() {
        let tokens: Vec<_> =
            Lexer::with_options(INDENTED_SRC, LexerOptions { ignore_indent: true })
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
        assert!(
            !tokens
                .iter()
                .any(|t| matches!(t.kind, TokenKind::Indent | TokenKind::Dedent)),
            "expected no Indent/Dedent with ignore_indent, got: {:?}",
            tokens.iter().map(|t| t.kind).collect::<Vec<_>>()
        );
    }

    #[test]
    fn env_truthy_enables_only_on_recognized_values() {
        use std::ffi::OsString;
        let v = |s: &str| env_truthy(Some(OsString::from(s)));
        // Recognized truthy values turn the flag on.
        assert!(v("1"));
        assert!(v("true"));
        assert!(v("yes"));
        // Everything else leaves it off, including unset, empty, and near-misses.
        assert!(!env_truthy(None));
        assert!(!v(""));
        assert!(!v("0"));
        assert!(!v("false"));
        assert!(!v("no"));
        assert!(!v("TRUE")); // exact match only — case-sensitive by design
    }
}
