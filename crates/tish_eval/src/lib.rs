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
pub use value::PropMap;
pub use value::Value;

/// Trait for pluggable native modules (e.g. Polars). Implement to register
/// globals with the interpreter. Return a map of (global_name, Value).
pub trait TishNativeModule: Send + Sync {
    fn name(&self) -> &'static str;
    fn register(&self) -> std::collections::HashMap<std::sync::Arc<str>, Value>;

    /// Virtual `tish:*` modules for `import { x } from 'tish:…'` (e.g. `tish:polars`).
    /// Return `(specifier, exports_object)` pairs. Default: none.
    fn virtual_builtin_modules(&self) -> Vec<(&'static str, Value)> {
        vec![]
    }
}
#[cfg(feature = "regex")]
pub use regex::TishRegExp;

pub fn run(source: &str) -> Result<Value, String> {
    let program = tishlang_parser::parse(source)?;
    let mut eval = Evaluator::new();
    let result = eval.eval_program(&program)?;
    #[cfg(feature = "http")]
    eval.run_timer_phase()?;
    Ok(result)
}

/// Run a Tish file with import/export support. Resolves relative imports from the file's directory.
/// Format an interpreter value for console output (Node/Bun-style colors when `colors` is true).
pub fn format_value_for_console(value: &Value, colors: bool) -> String {
    match value_convert::eval_to_core(value) {
        Ok(core_val) => tishlang_core::format_value_styled(&core_val, colors),
        Err(_) => value.to_string(),
    }
}

/// Run a Tish file with import/export support. Resolves relative imports from the file's directory.
pub fn run_file(path: &std::path::Path, project_root: Option<&std::path::Path>) -> Result<Value, String> {
    let path = path
        .canonicalize()
        .map_err(|e| format!("Cannot canonicalize {}: {}", path.display(), e))?;
    let source = std::fs::read_to_string(&path).map_err(|e| format!("Cannot read {}: {}", path.display(), e))?;
    let program = tishlang_parser::parse(&source)?;
    let mut eval = Evaluator::new();
    eval.set_current_dir(project_root.or(path.parent()));
    let result = eval.eval_program(&program)?;
    #[cfg(feature = "http")]
    eval.run_timer_phase()?;
    Ok(result)
}
