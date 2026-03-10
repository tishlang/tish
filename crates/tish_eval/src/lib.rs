//! Tish tree-walk interpreter.

mod eval;
#[cfg(feature = "http")]
mod http;
mod value_convert;
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

/// Trait for pluggable native modules (e.g. Polars). Implement to register
/// globals with the interpreter. Return a map of (global_name, Value).
pub trait TishNativeModule: Send + Sync {
    fn name(&self) -> &'static str;
    fn register(&self) -> std::collections::HashMap<std::sync::Arc<str>, Value>;
}
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

/// Run a Tish file with import/export support. Resolves relative imports from the file's directory.
pub fn run_file(path: &std::path::Path, project_root: Option<&std::path::Path>) -> Result<Value, String> {
    let path = path
        .canonicalize()
        .map_err(|e| format!("Cannot canonicalize {}: {}", path.display(), e))?;
    let source = std::fs::read_to_string(&path).map_err(|e| format!("Cannot read {}: {}", path.display(), e))?;
    let program = tish_parser::parse(&source)?;
    let mut eval = Evaluator::new();
    eval.set_current_dir(project_root.or(path.parent()));
    let result = eval.eval_program(&program)?;
    #[cfg(feature = "http")]
    eval.run_timer_phase()?;
    Ok(result)
}
