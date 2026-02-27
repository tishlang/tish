//! Tish tree-walk interpreter.

mod eval;
#[cfg(feature = "http")]
mod http;
mod value;

pub use eval::Evaluator;
pub use value::Value;

pub fn run(source: &str) -> Result<Value, String> {
    let program = tish_parser::parse(source)?;
    let mut eval = Evaluator::new();
    eval.eval_program(&program)
}
