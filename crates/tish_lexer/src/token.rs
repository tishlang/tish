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
    Let,
    Const,
    If,
    Else,
    While,
    For,
    Return,
    Break,
    Continue,
    Throw,
    Try,
    Catch,
    Switch,
    Case,
    Default,
    Do,
    TypeOf,
    Void,
    Of,
    In,

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
    Spread,
    Colon,

    // Operators
    Assign,
    PlusAssign,
    MinusAssign,
    StarAssign,
    SlashAssign,
    PercentAssign,
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
    PlusPlus,
    MinusMinus,
    Star,
    StarStar,
    Slash,
    Percent,
    And,
    Or,
    Not,
    BitAnd,
    BitOr,
    BitXor,
    BitNot,
    Shl,
    Shr,
    OptionalChain,
    NullishCoalesce,
    Question,
    Arrow,
    
    // Template literal tokens
    TemplateNoSub,   // `text` (no interpolation)
    TemplateHead,    // `text${  (start with interpolation)
    TemplateMiddle,  // }text${  (middle part)
    TemplateTail,    // }text`   (end part)

    Eof,
}

impl TokenKind {
    pub fn keyword_or_ident(s: &str) -> Self {
        match s {
            "fun" => TokenKind::Fun,
            "let" => TokenKind::Let,
            "const" => TokenKind::Const,
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
            "throw" => TokenKind::Throw,
            "try" => TokenKind::Try,
            "catch" => TokenKind::Catch,
            "switch" => TokenKind::Switch,
            "case" => TokenKind::Case,
            "default" => TokenKind::Default,
            "do" => TokenKind::Do,
            "typeof" => TokenKind::TypeOf,
            "void" => TokenKind::Void,
            "of" => TokenKind::Of,
            "in" => TokenKind::In,
            _ => TokenKind::Ident,
        }
    }
}
