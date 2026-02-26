//! Tish tree-walk interpreter.

mod eval;
mod value;

pub use eval::Evaluator;
pub use value::Value;

use tish_ast::Program;
use tish_parser;

pub fn run(source: &str) -> Result<Value, String> {
    let program = tish_parser::parse(source)?;
    let mut eval = Evaluator::new();
    eval.eval_program(&program)
}
