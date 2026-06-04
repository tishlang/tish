#[cfg(test)]
mod issue_1_hex_binary_literals {
    use tish_lexer::{Lexer, TokenKind};

    #[test]
    fn test_hex_literal_uppercase() {
        let source = "let v = 0xFF";
        let tokens: Result<Vec<_>, _> = Lexer::new(source).collect();
        let tokens = tokens.expect("lexing should succeed");
        
        // Find the numeric token
        let num_token = tokens.iter()
            .find(|t| matches!(t.kind, TokenKind::Number))
            .expect("should have a Number token");
        
        assert_eq!(num_token.literal.as_deref(), Some("0xFF"), 
            "hex literal should be lexed as a single Number token, not split into '0' and identifier 'xFF'");
    }

    #[test]
    fn test_hex_literal_lowercase() {
        let source = "let v = 0xff";
        let tokens: Result<Vec<_>, _> = Lexer::new(source).collect();
        let tokens = tokens.expect("lexing should succeed");
        
        let num_token = tokens.iter()
            .find(|t| matches!(t.kind, TokenKind::Number))
            .expect("should have a Number token");
        
        assert_eq!(num_token.literal.as_deref(), Some("0xff"), 
            "hex literal 0xff should be lexed as a single Number token");
    }

    #[test]
    fn test_hex_literal_long() {
        let source = "let v = 0xFFFFFF";
        let tokens: Result<Vec<_>, _> = Lexer::new(source).collect();
        let tokens = tokens.expect("lexing should succeed");
        
        let num_token = tokens.iter()
            .find(|t| matches!(t.kind, TokenKind::Number))
            .expect("should have a Number token");
        
        assert_eq!(num_token.literal.as_deref(), Some("0xFFFFFF"), 
            "hex literal 0xFFFFFF should be lexed as a single Number token");
    }

    #[test]
    fn test_binary_literal() {
        let source = "let v = 0b1010";
        let tokens: Result<Vec<_>, _> = Lexer::new(source).collect();
        let tokens = tokens.expect("lexing should succeed");
        
        let num_token = tokens.iter()
            .find(|t| matches!(t.kind, TokenKind::Number))
            .expect("should have a Number token");
        
        assert_eq!(num_token.literal.as_deref(), Some("0b1010"), 
            "binary literal should be lexed as a single Number token, not split into '0' and identifier 'b1010'");
    }

    #[test]
    fn test_decimal_literal_still_works() {
        let source = "let v = 255";
        let tokens: Result<Vec<_>, _> = Lexer::new(source).collect();
        let tokens = tokens.expect("lexing should succeed");
        
        let num_token = tokens.iter()
            .find(|t| matches!(t.kind, TokenKind::Number))
            .expect("should have a Number token");
        
        assert_eq!(num_token.literal.as_deref(), Some("255"), 
            "decimal literal should still work");
    }

    #[test]
    fn test_hex_not_split_into_identifier() {
        // This test specifically checks that we DON'T get an identifier token
        // after the number, which is the current bug behavior
        let source = "let v = 0xFF";
        let tokens: Result<Vec<_>, _> = Lexer::new(source).collect();
        let tokens = tokens.expect("lexing should succeed");
        
        // Count identifier tokens that look like hex suffixes
        let bad_ident = tokens.iter()
            .any(|t| matches!(t.kind, TokenKind::Ident) && 
                     t.literal.as_deref().map_or(false, |s| s == "xFF" || s == "xff" || s.starts_with('x')));
        
        assert!(!bad_ident, 
            "should not have an identifier token 'xFF' or similar - the entire 0xFF should be one Number token");
    }

    #[test]
    fn test_binary_not_split_into_identifier() {
        let source = "let v = 0b1010";
        let tokens: Result<Vec<_>, _> = Lexer::new(source).collect();
        let tokens = tokens.expect("lexing should succeed");
        
        let bad_ident = tokens.iter()
            .any(|t| matches!(t.kind, TokenKind::Ident) && 
                     t.literal.as_deref().map_or(false, |s| s == "b1010" || s.starts_with('b')));
        
        assert!(!bad_ident, 
            "should not have an identifier token 'b1010' - the entire 0b1010 should be one Number token");
    }
}
