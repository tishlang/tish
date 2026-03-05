//! Tish recursive descent parser.

mod parser;

use parser::Parser;

use tish_ast::Program;
use tish_lexer::Lexer;

pub fn parse(source: &str) -> Result<Program, String> {
    let lexer = Lexer::new(source);
    let tokens: Result<Vec<_>, _> = lexer.collect();
    let tokens = tokens?;
    let mut parser = Parser::new(&tokens);
    parser.parse_program()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_async_fn_parse() {
        let program = parse("async fn foo() { }").expect("parse async fn");
        assert_eq!(program.statements.len(), 1);
        if let tish_ast::Statement::FunDecl { async_, name, .. } = &program.statements[0] {
            assert!(async_, "expected async function");
            assert_eq!(name.as_ref(), "foo");
        } else {
            panic!("expected FunDecl");
        }
    }
}
