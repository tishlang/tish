//! Token types for the Tish lexer.

use std::sync::Arc;

#[derive(Debug, Clone, PartialEq)]
pub struct Span {
    pub start: (usize, usize), // line, col
    pub end: (usize, usize),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
    pub literal: Option<Arc<str>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    // Virtual tokens for optional braces
    Indent,
    Dedent,

    // Literals
    Number,
    String,
    True,
    False,
    Null,

    // Identifiers and keywords
    Ident,
    Fun,
    Any,
    If,
    Else,
    While,
    For,
    Return,
    Break,
    Continue,

    // Punctuation
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Semicolon,
    Comma,
    Dot,
    Colon,

    // Operators
    Assign,
    Eq,
    Ne,
    StrictEq,
    StrictNe,
    Lt,
    Le,
    Gt,
    Ge,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    And,
    Or,
    Not,
    BitAnd,
    BitOr,
    OptionalChain,
    NullishCoalesce,
    Question,

    Eof,
}

impl TokenKind {
    pub fn keyword_or_ident(s: &str) -> Self {
        match s {
            "fun" => TokenKind::Fun,
            "any" => TokenKind::Any,
            "if" => TokenKind::If,
            "else" => TokenKind::Else,
            "while" => TokenKind::While,
            "for" => TokenKind::For,
            "return" => TokenKind::Return,
            "break" => TokenKind::Break,
            "continue" => TokenKind::Continue,
            "true" => TokenKind::True,
            "false" => TokenKind::False,
            "null" => TokenKind::Null,
            _ => TokenKind::Ident,
        }
    }

    pub fn is_eof(&self) -> bool {
        matches!(self, TokenKind::Eof)
    }
}
