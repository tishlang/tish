//! Tish to JavaScript transpiler backend.
//! Uses shared resolve from tish_compile for unified pipeline.

mod codegen;
mod error;
mod js_intrinsics;

#[cfg(test)]
mod tests_jsx;

pub use codegen::{compile_project_with_jsx, compile_with_jsx, JsxMode};
pub use error::CompileError;

/// Default entry: Tishact `h(tag, props, [children])` (import `h` + `Fragment` from Tishact.tish; entry should import Tishact before other UI).
pub fn compile(program: &tish_ast::Program, optimize: bool) -> Result<String, CompileError> {
    compile_with_jsx(program, optimize, JsxMode::TishactH)
}

pub fn compile_project(
    entry_path: &std::path::Path,
    project_root: Option<&std::path::Path>,
    optimize: bool,
) -> Result<String, CompileError> {
    compile_project_with_jsx(entry_path, project_root, optimize, JsxMode::TishactH)
}
