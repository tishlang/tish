//! Tish lexer with indent normalization and tab/space handling.
//!
//! Normalizes tabs and spaces to a single indent level so both styles work.
//! Emits virtual Indent/Dedent tokens for optional-brace blocks.

mod token;

pub use token::{Token, TokenKind, Span};

use std::collections::VecDeque;
use std::iter::Peekable;
use std::str::Chars;

const INDENT_WIDTH: usize = 2; // spaces per indent level
const TAB_AS_LEVELS: usize = 1; // 1 tab = 1 indent level

#[derive(Debug, Clone)]
pub struct Lexer<'a> {
    source: &'a str,
    chars: Peekable<Chars<'a>>,
    pos: usize,
    line: usize,
    col: usize,
    indent_stack: Vec<usize>,
    at_line_start: bool,
    pending_dedents: VecDeque<Token>,
}

impl<'a> Lexer<'a> {
    pub fn new(source: &'a str) -> Self {
        let mut lexer = Self {
            source,
            chars: source.chars().peekable(),
            pos: 0,
            line: 1,
            col: 1,
            indent_stack: vec![0],
            at_line_start: true,
            pending_dedents: VecDeque::new(),
        };
        lexer
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

    /// Convert leading whitespace to logical indent level.
    /// Tab = 1 level; N spaces = 1 level (INDENT_WIDTH spaces per level).
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
        // Normalize: N spaces = 1 level (round down)
        (level + INDENT_WIDTH - 1) / INDENT_WIDTH
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
        let mut s = String::from(first);
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

    fn read_string(&mut self, quote: char) -> Result<String, String> {
        let mut s = String::new();
        // Opening quote already consumed by next_token
        loop {
            match self.advance() {
                None => return Err("Unterminated string".to_string()),
                Some(c) if c == quote => break,
                Some('\\') => {
                    let escaped = self.advance().ok_or("Unterminated escape")?;
                    let c = match escaped {
                        'n' => '\n',
                        'r' => '\r',
                        't' => '\t',
                        '\\' => '\\',
                        '"' => '"',
                        '\'' => '\'',
                        _ => return Err(format!("Unknown escape: \\{}", escaped)),
                    };
                    s.push(c);
                }
                Some(c) => s.push(c),
            }
        }
        Ok(s)
    }

    fn read_ident_or_keyword(&mut self, first: char) -> String {
        let mut s = String::from(first);
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

    fn emit_indent_or_dedent(&mut self, level: usize) -> Option<Token> {
        let top = *self.indent_stack.last().unwrap();
        let start = self.span_start();

        if level > top {
            self.indent_stack.push(level);
            Some(Token {
                kind: TokenKind::Indent,
                span: Span {
                    start: (start.0, start.1),
                    end: (start.0, start.1),
                },
                literal: None,
            })
        } else if level < top {
            // Pop and emit one Dedent per level; return first, queue rest
            while self.indent_stack.len() > 1 && *self.indent_stack.last().unwrap() > level {
                self.indent_stack.pop();
                let dedent = Token {
                    kind: TokenKind::Dedent,
                    span: Span {
                        start: (start.0, start.1),
                        end: (start.0, start.1),
                    },
                    literal: None,
                };
                self.pending_dedents.push_back(dedent);
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
        // Drain pending dedents first
        if let Some(tok) = self.pending_dedents.pop_front() {
            return Ok(Some(tok));
        }

        // At line start: handle indentation
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
                // Emit pending dedents at EOF
                if let Some(tok) = self.pending_dedents.pop_front() {
                    return Ok(Some(tok));
                }
                while self.indent_stack.len() > 1 {
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
            '{' => TokenKind::LBrace,
            '}' => TokenKind::RBrace,
            '[' => TokenKind::LBracket,
            ']' => TokenKind::RBracket,
            ';' => TokenKind::Semicolon,
            ',' => TokenKind::Comma,
            '.' => {
                if self.peek() == Some('?') {
                    self.advance();
                    TokenKind::OptionalChain
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
                } else {
                    TokenKind::Lt
                }
            }
            '>' => {
                if self.peek() == Some('=') {
                    self.advance();
                    TokenKind::Ge
                } else {
                    TokenKind::Gt
                }
            }
            '+' => TokenKind::Plus,
            '-' => TokenKind::Minus,
            '*' => TokenKind::Star,
            '/' => {
                if self.peek() == Some('/') {
                    self.advance();
                    self.skip_line_comment();
                    return self.next_token();
                } else if self.peek() == Some('*') {
                    self.advance();
                    self.skip_block_comment()?;
                    return self.next_token();
                } else {
                    TokenKind::Slash
                }
            }
            '%' => TokenKind::Percent,
            '&' => {
                if self.peek() == Some('&') {
                    self.advance();
                    TokenKind::And
                } else {
                    TokenKind::BitAnd
                }
            }
            '|' => {
                if self.peek() == Some('|') {
                    self.advance();
                    TokenKind::Or
                } else {
                    TokenKind::BitOr
                }
            }
            '?' => {
                if self.peek() == Some('?') {
                    self.advance();
                    TokenKind::NullishCoalesce
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
                    literal: if matches!(kind, TokenKind::Ident) {
                        Some(ident.into())
                    } else {
                        None
                    },
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
}
