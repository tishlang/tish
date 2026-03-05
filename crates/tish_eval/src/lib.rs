//! Tish tree-walk interpreter.

mod eval;
#[cfg(feature = "http")]
mod http;
#[cfg(feature = "http")]
mod promise;
#[cfg(feature = "http")]
mod timers;
mod natives;
#[cfg(feature = "regex")]
pub mod regex;
mod value;

pub use eval::Evaluator;
pub use value::Value;
#[cfg(feature = "regex")]
pub use regex::TishRegExp;

pub fn run(source: &str) -> Result<Value, String> {
    let program = tish_parser::parse(source)?;
    let mut eval = Evaluator::new();
    let result = eval.eval_program(&program)?;
    #[cfg(feature = "http")]
    eval.run_timer_phase()?;
    Ok(result)
}
