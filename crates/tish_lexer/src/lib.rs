//! Tish lexer with indent normalization and tab/space handling.
//!
//! Normalizes tabs and spaces to a single indent level so both styles work.
//! Emits virtual Indent/Dedent tokens for optional-brace blocks.

mod token;

pub use token::{Token, TokenKind, Span};

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
}

impl<'a> Lexer<'a> {
    pub fn new(source: &'a str) -> Self {
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
        }
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
                Some(c) => { self.advance(); s.push(c); }
            }
        }
        if s.is_empty() {
            Ok(None)
        } else {
            let end = self.span_start();
            Ok(Some(Token { kind: TokenKind::JsxText, span: Span { start, end }, literal: Some(s.into()) }))
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
            if c == '\n' { break; }
        }
    }

    fn skip_block_comment(&mut self) -> Result<(), String> {
        let mut depth = 1;
        while depth > 0 {
            match self.advance() {
                Some('*') if self.peek() == Some('/') => { self.advance(); depth -= 1; }
                Some('/') if self.peek() == Some('*') => { self.advance(); depth += 1; }
                None => return Err("Unterminated block comment".to_string()),
                _ => {}
            }
        }
        Ok(())
    }

    fn read_number(&mut self, first: char) -> String {
        let mut s = String::with_capacity(16);
        s.push(first);
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() || c == '.' {
                s.push(c);
                self.advance();
            } else {
                break;
            }
        }
        s
    }

    /// Handle escape sequence, returning the unescaped character.
    /// `extra_allowed` contains additional characters that can be escaped in this context.
    fn handle_escape(&mut self, extra_allowed: &[char]) -> Result<char, String> {
        let escaped = self.advance().ok_or("Unterminated escape")?;
        match escaped {
            'n' => Ok('\n'),
            'r' => Ok('\r'),
            't' => Ok('\t'),
            '\\' => Ok('\\'),
            c if extra_allowed.contains(&c) => Ok(c),
            _ => Err(format!("Unknown escape: \\{}", escaped)),
        }
    }

    fn read_string(&mut self, quote: char) -> Result<String, String> {
        let mut s = String::with_capacity(32);
        let extra = if quote == '"' { &['"', '\''][..] } else { &['\'', '"'][..] };
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
    fn read_template(&mut self, start: (usize, usize), is_continuation: bool) -> Result<Option<Token>, String> {
        let mut s = String::with_capacity(if is_continuation { 32 } else { 64 });
        let extra = &['`', '$', '{'][..];
        
        loop {
            match self.advance() {
                None => return Err("Unterminated template literal".to_string()),
                Some('`') => {
                    let end = self.span_start();
                    let kind = if is_continuation { TokenKind::TemplateTail } else { TokenKind::TemplateNoSub };
                    return Ok(Some(Token { kind, span: Span { start, end }, literal: Some(s.into()) }));
                }
                Some('$') if self.peek() == Some('{') => {
                    self.advance();
                    self.template_brace_stack.push(1);
                    let end = self.span_start();
                    let kind = if is_continuation { TokenKind::TemplateMiddle } else { TokenKind::TemplateHead };
                    return Ok(Some(Token { kind, span: Span { start, end }, literal: Some(s.into()) }));
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
            let level = self.read_indent_level();
            if level > 0 || self.peek().map(|c| c != '\n').unwrap_or(false) {
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
                        span: Span { start: (self.line, self.col), end: (self.line, self.col) },
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
                if self.peek() == Some('?') { self.advance(); TokenKind::OptionalChain }
                else if self.peek() == Some('.') {
                    self.advance();
                    if self.peek() == Some('.') { self.advance(); TokenKind::Spread }
                    else { return Err("Unexpected .. (use ... for rest params)".to_string()); }
                } else { TokenKind::Dot }
            }
            '=' => {
                if self.peek() == Some('=') {
                    self.advance();
                    if self.peek() == Some('=') { self.advance(); TokenKind::StrictEq } else { TokenKind::Eq }
                } else if self.peek() == Some('>') { self.advance(); TokenKind::Arrow }
                else { TokenKind::Assign }
            }
            '!' => {
                if self.peek() == Some('=') {
                    self.advance();
                    if self.peek() == Some('=') { self.advance(); TokenKind::StrictNe } else { TokenKind::Ne }
                } else { TokenKind::Not }
            }
            '<' => {
                if self.peek() == Some('=') { self.advance(); TokenKind::Le }
                else if self.peek() == Some('<') { self.advance(); TokenKind::Shl }
                else if self.peek() == Some('/') { self.jsx_in_closing_tag = true; TokenKind::Lt }
                else if self.peek() == Some('>') || self.peek().map(|c| c.is_ascii_alphabetic() || c == '_').unwrap_or(false) {
                    self.jsx_depth += 1;
                    self.jsx_stack.push(JsxEl {
                        in_opener: true,
                        attr_value_braces: 0,
                    });
                    self.jsx_in_opening_tag = true;
                    TokenKind::Lt
                } else { TokenKind::Lt }
            }
            '>' => {
                if self.peek() == Some('=') { self.advance(); TokenKind::Ge }
                else if self.peek() == Some('>') { self.advance(); TokenKind::Shr }
                else {
                    if self.jsx_in_closing_tag {
                        self.jsx_depth = (self.jsx_depth - 1).max(0);
                        self.jsx_stack.pop();
                        self.jsx_sync_in_opening_tag();
                    } else if self.jsx_in_opening_tag && self.jsx_saw_slash_before_gt {
                        self.jsx_depth = (self.jsx_depth - 1).max(0);
                        self.jsx_stack.pop();
                        self.jsx_sync_in_opening_tag();
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
                if self.peek() == Some('+') { self.advance(); TokenKind::PlusPlus }
                else if self.peek() == Some('=') { self.advance(); TokenKind::PlusAssign }
                else { TokenKind::Plus }
            }
            '-' => {
                if self.peek() == Some('-') { self.advance(); TokenKind::MinusMinus }
                else if self.peek() == Some('=') { self.advance(); TokenKind::MinusAssign }
                else { TokenKind::Minus }
            }
            '*' => {
                if self.peek() == Some('*') { self.advance(); TokenKind::StarStar }
                else if self.peek() == Some('=') { self.advance(); TokenKind::StarAssign }
                else { TokenKind::Star }
            }
            '/' => {
                if self.peek() == Some('/') { self.advance(); self.skip_line_comment(); return self.next_token(); }
                else if self.peek() == Some('*') { self.advance(); self.skip_block_comment()?; return self.next_token(); }
                else if self.peek() == Some('=') { self.advance(); TokenKind::SlashAssign }
                else {
                    if self.jsx_in_opening_tag { self.jsx_saw_slash_before_gt = true; }
                    TokenKind::Slash
                }
            }
            '%' => {
                if self.peek() == Some('=') { self.advance(); TokenKind::PercentAssign }
                else { TokenKind::Percent }
            }
            '&' => {
                if self.peek() == Some('&') {
                    self.advance();
                    if self.peek() == Some('=') { self.advance(); TokenKind::AndAndAssign }
                    else { TokenKind::And }
                } else { TokenKind::BitAnd }
            }
            '|' => {
                if self.peek() == Some('|') {
                    self.advance();
                    if self.peek() == Some('=') { self.advance(); TokenKind::OrOrAssign }
                    else { TokenKind::Or }
                } else { TokenKind::BitOr }
            }
            '?' => {
                if self.peek() == Some('?') {
                    self.advance();
                    if self.peek() == Some('=') { self.advance(); TokenKind::NullishAssign }
                    else { TokenKind::NullishCoalesce }
                } else if self.peek() == Some('.') { self.advance(); TokenKind::OptionalChain }
                else { TokenKind::Question }
            }
            ':' => TokenKind::Colon,
            '"' | '\'' => {
                let s = self.read_string(c)?;
                let end = self.span_start();
                return Ok(Some(Token { kind: TokenKind::String, span: Span { start, end }, literal: Some(s.into()) }));
            }
            '`' => return self.read_template(start, false),
            '0'..='9' => {
                let num = self.read_number(c);
                let end = self.span_start();
                return Ok(Some(Token { kind: TokenKind::Number, span: Span { start, end }, literal: Some(num.into()) }));
            }
            'a'..='z' | 'A'..='Z' | '_' => {
                let ident = self.read_ident_or_keyword(c);
                let end = self.span_start();
                let kind = TokenKind::keyword_or_ident(&ident);
                return Ok(Some(Token {
                    kind,
                    span: Span { start, end },
                    literal: if matches!(kind, TokenKind::Ident) { Some(ident.into()) } else { None },
                }));
            }
            '\n' => { self.at_line_start = true; return self.next_token(); }
            _ => return Err(format!("Unexpected character: {:?}", c)),
        };

        let end = self.span_start();
        Ok(Some(Token { kind, span: Span { start, end }, literal: None }))
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
}
