//! Regression test for issue #1: hex and binary numeric literals don't parse
//!
//! Bug: The lexer splits `0xFF` into `0` (Number) and `xFF` (Ident), causing
//! "Undefined variable: xFF" errors. Same for binary literals like `0b1010`.
//!
//! Expected: `0xFF` should lex as a single Number token with literal "0xFF" (or "255").
//! Expected: `0b1010` should lex as a single Number token with literal "0b1010" (or "10").

use tishlang_lexer::{Lexer, TokenKind};

#[test]
fn hex_literal_uppercase_parses_as_single_number() {
    let source = "let v = 0xFF";
    let tokens: Result<Vec<_>, _> = Lexer::new(source).collect();
    let tokens = tokens.expect("lexing should succeed");

    // Find the Number token
    let number_tokens: Vec<_> = tokens
        .iter()
        .filter(|t| t.kind == TokenKind::Number)
        .collect();

    assert_eq!(
        number_tokens.len(),
        1,
        "Expected exactly one Number token, but got tokens: {:?}",
        tokens.iter().map(|t| &t.kind).collect::<Vec<_>>()
    );

    let num_tok = number_tokens[0];
    let literal = num_tok.literal.as_deref().expect("Number should have literal");

    // The lexer should recognize 0xFF as a hex literal
    assert!(
        literal == "0xFF" || literal == "255",
        "Expected literal '0xFF' or '255', got '{}'",
        literal
    );
}

#[test]
fn hex_literal_lowercase_parses_as_single_number() {
    let source = "let v = 0xff";
    let tokens: Result<Vec<_>, _> = Lexer::new(source).collect();
    let tokens = tokens.expect("lexing should succeed");

    let number_tokens: Vec<_> = tokens
        .iter()
        .filter(|t| t.kind == TokenKind::Number)
        .collect();

    assert_eq!(
        number_tokens.len(),
        1,
        "Expected exactly one Number token, but got tokens: {:?}",
        tokens.iter().map(|t| &t.kind).collect::<Vec<_>>()
    );

    let num_tok = number_tokens[0];
    let literal = num_tok.literal.as_deref().expect("Number should have literal");

    assert!(
        literal == "0xff" || literal == "255",
        "Expected literal '0xff' or '255', got '{}'",
        literal
    );
}

#[test]
fn hex_literal_long_parses_as_single_number() {
    let source = "let v = 0xFFFFFF";
    let tokens: Result<Vec<_>, _> = Lexer::new(source).collect();
    let tokens = tokens.expect("lexing should succeed");

    let number_tokens: Vec<_> = tokens
        .iter()
        .filter(|t| t.kind == TokenKind::Number)
        .collect();

    assert_eq!(
        number_tokens.len(),
        1,
        "Expected exactly one Number token, but got tokens: {:?}",
        tokens.iter().map(|t| &t.kind).collect::<Vec<_>>()
    );

    let num_tok = number_tokens[0];
    let literal = num_tok.literal.as_deref().expect("Number should have literal");

    assert!(
        literal == "0xFFFFFF" || literal == "16777215",
        "Expected literal '0xFFFFFF' or '16777215', got '{}'",
        literal
    );
}

#[test]
fn binary_literal_parses_as_single_number() {
    let source = "let v = 0b1010";
    let tokens: Result<Vec<_>, _> = Lexer::new(source).collect();
    let tokens = tokens.expect("lexing should succeed");

    let number_tokens: Vec<_> = tokens
        .iter()
        .filter(|t| t.kind == TokenKind::Number)
        .collect();

    assert_eq!(
        number_tokens.len(),
        1,
        "Expected exactly one Number token, but got tokens: {:?}",
        tokens.iter().map(|t| &t.kind).collect::<Vec<_>>()
    );

    let num_tok = number_tokens[0];
    let literal = num_tok.literal.as_deref().expect("Number should have literal");

    assert!(
        literal == "0b1010" || literal == "10",
        "Expected literal '0b1010' or '10', got '{}'",
        literal
    );
}

#[test]
fn decimal_literal_still_works() {
    let source = "let v = 255";
    let tokens: Result<Vec<_>, _> = Lexer::new(source).collect();
    let tokens = tokens.expect("lexing should succeed");

    let number_tokens: Vec<_> = tokens
        .iter()
        .filter(|t| t.kind == TokenKind::Number)
        .collect();

    assert_eq!(number_tokens.len(), 1);

    let num_tok = number_tokens[0];
    let literal = num_tok.literal.as_deref().expect("Number should have literal");

    assert_eq!(literal, "255");
}

#[test]
fn hex_literal_not_mistaken_for_identifier() {
    // This is the core bug: 0xFF should NOT produce an Ident token for "xFF"
    let source = "0xFF";
    let tokens: Result<Vec<_>, _> = Lexer::new(source).collect();
    let tokens = tokens.expect("lexing should succeed");

    let ident_tokens: Vec<_> = tokens
        .iter()
        .filter(|t| t.kind == TokenKind::Ident)
        .collect();

    assert_eq!(
        ident_tokens.len(),
        0,
        "0xFF should not produce any Ident tokens, but got: {:?}",
        ident_tokens
            .iter()
            .map(|t| t.literal.as_deref())
            .collect::<Vec<_>>()
    );
}

#[test]
fn binary_literal_not_mistaken_for_identifier() {
    let source = "0b1010";
    let tokens: Result<Vec<_>, _> = Lexer::new(source).collect();
    let tokens = tokens.expect("lexing should succeed");

    let ident_tokens: Vec<_> = tokens
        .iter()
        .filter(|t| t.kind == TokenKind::Ident)
        .collect();

    assert_eq!(
        ident_tokens.len(),
        0,
        "0b1010 should not produce any Ident tokens, but got: {:?}",
        ident_tokens
            .iter()
            .map(|t| t.literal.as_deref())
            .collect::<Vec<_>>()
    );
}

#[test]
fn hex_literal_in_expression() {
    // Test from the issue description: inside parens after &
    let source = "let mask = (0xFF & value)";
    let tokens: Result<Vec<_>, _> = Lexer::new(source).collect();
    let tokens = tokens.expect("lexing should succeed");

    let number_tokens: Vec<_> = tokens
        .iter()
        .filter(|t| t.kind == TokenKind::Number)
        .collect();

    assert_eq!(
        number_tokens.len(),
        1,
        "Expected exactly one Number token for 0xFF"
    );

    // Should not have spurious identifiers like "xFF"
    let spurious_idents: Vec<_> = tokens
        .iter()
        .filter(|t| {
            t.kind == TokenKind::Ident
                && t.literal
                    .as_deref()
                    .map(|s| s.starts_with('x') || s.starts_with('b'))
                    .unwrap_or(false)
        })
        .collect();

    assert_eq!(
        spurious_idents.len(),
        0,
        "Should not have identifiers starting with 'x' or 'b' from hex/binary literals"
    );
}
