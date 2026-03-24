//! Tish to JavaScript transpiler backend.
//! Uses shared resolve from tishlang_compile for unified pipeline.

mod codegen;
mod error;
mod js_intrinsics;

#[cfg(test)]
mod tests_jsx;

pub use codegen::{compile_project_with_jsx, compile_with_jsx};
pub use error::CompileError;

/// JSX lowers to `h` / `Fragment`; merge the `lattish` runtime for hooks and DOM.
pub fn compile(program: &tishlang_ast::Program, optimize: bool) -> Result<String, CompileError> {
    compile_with_jsx(program, optimize)
}

pub fn compile_project(
    entry_path: &std::path::Path,
    project_root: Option<&std::path::Path>,
    optimize: bool,
) -> Result<String, CompileError> {
    compile_project_with_jsx(entry_path, project_root, optimize)
}
