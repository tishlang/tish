//! Tish recursive descent parser.

mod parser;

pub use parser::Parser;

use tish_ast::Program;
use tish_lexer::Lexer;

pub fn parse(source: &str) -> Result<Program, String> {
    let lexer = Lexer::new(source);
    let tokens: Result<Vec<_>, _> = lexer.collect();
    let tokens = tokens?;
    let mut parser = Parser::new(&tokens);
    parser.parse_program()
}
