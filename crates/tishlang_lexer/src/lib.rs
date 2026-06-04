// Tish Lexer Implementation
// Handles tokenization with proper hex and binary literal support

use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    Number,
    Identifier,
    Keyword,
    Operator,
    Punctuation,
    String,
    Comment,
    Whitespace,
    Newline,
    Indent,
    Dedent,
    Eof,
}

#[derive(Debug, Clone)]
pub struct Token {
    pub kind: TokenKind,
    pub lexeme: String,
    pub line: usize,
    pub column: usize,
}

pub struct Lexer {
    source: Vec<char>,
    position: usize,
    line: usize,
    column: usize,
}

impl Lexer {
    pub fn new(source: &str) -> Self {
        Self {
            source: source.chars().collect(),
            position: 0,
            line: 1,
            column: 1,
        }
    }

    fn current(&self) -> Option<char> {
        self.source.get(self.position).copied()
    }

    fn peek(&self, offset: usize) -> Option<char> {
        self.source.get(self.position + offset).copied()
    }

    fn advance(&mut self) -> Option<char> {
        let ch = self.current()?;
        self.position += 1;
        if ch == '\n' {
            self.line += 1;
            self.column = 1;
        } else {
            self.column += 1;
        }
        Some(ch)
    }

    fn scan_number(&mut self) -> Token {
        let start_line = self.line;
        let start_column = self.column;
        let mut lexeme = String::new();

        // Check for hex (0x) or binary (0b) prefix
        if self.current() == Some('0') {
            if let Some(next) = self.peek(1) {
                if next == 'x' || next == 'X' {
                    // Hex literal
                    lexeme.push(self.advance().unwrap()); // '0'
                    lexeme.push(self.advance().unwrap()); // 'x' or 'X'
                    
                    // Scan hex digits
                    while let Some(ch) = self.current() {
                        if ch.is_ascii_hexdigit() {
                            lexeme.push(self.advance().unwrap());
                        } else {
                            break;
                        }
                    }
                    
                    return Token {
                        kind: TokenKind::Number,
                        lexeme,
                        line: start_line,
                        column: start_column,
                    };
                } else if next == 'b' || next == 'B' {
                    // Binary literal
                    lexeme.push(self.advance().unwrap()); // '0'
                    lexeme.push(self.advance().unwrap()); // 'b' or 'B'
                    
                    // Scan binary digits
                    while let Some(ch) = self.current() {
                        if ch == '0' || ch == '1' {
                            lexeme.push(self.advance().unwrap());
                        } else {
                            break;
                        }
                    }
                    
                    return Token {
                        kind: TokenKind::Number,
                        lexeme,
                        line: start_line,
                        column: start_column,
                    };
                }
            }
        }

        // Regular decimal number
        while let Some(ch) = self.current() {
            if ch.is_ascii_digit() || ch == '.' || ch == '_' {
                lexeme.push(self.advance().unwrap());
            } else {
                break;
            }
        }

        Token {
            kind: TokenKind::Number,
            lexeme,
            line: start_line,
            column: start_column,
        }
    }

    fn scan_identifier(&mut self) -> Token {
        let start_line = self.line;
        let start_column = self.column;
        let mut lexeme = String::new();

        while let Some(ch) = self.current() {
            if ch.is_alphanumeric() || ch == '_' {
                lexeme.push(self.advance().unwrap());
            } else {
                break;
            }
        }

        Token {
            kind: TokenKind::Identifier,
            lexeme,
            line: start_line,
            column: start_column,
        }
    }

    fn skip_whitespace(&mut self) {
        while let Some(ch) = self.current() {
            if ch.is_whitespace() && ch != '\n' {
                self.advance();
            } else {
                break;
            }
        }
    }
}

impl Iterator for Lexer {
    type Item = Token;

    fn next(&mut self) -> Option<Self::Item> {
        self.skip_whitespace();

        let ch = self.current()?;
        let token = if ch.is_ascii_digit() {
            self.scan_number()
        } else if ch.is_alphabetic() || ch == '_' {
            self.scan_identifier()
        } else if ch == '\n' {
            let line = self.line;
            let column = self.column;
            self.advance();
            Token {
                kind: TokenKind::Newline,
                lexeme: "\n".to_string(),
                line,
                column,
            }
        } else {
            // Other tokens (operators, punctuation, etc.)
            let line = self.line;
            let column = self.column;
            let lexeme = self.advance().unwrap().to_string();
            Token {
                kind: TokenKind::Operator,
                lexeme,
                line,
                column,
            }
        };

        Some(token)
    }
}
