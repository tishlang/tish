//! Tish to JavaScript transpiler backend.
//! Uses shared resolve from tishlang_compile for unified pipeline.

mod codegen;
mod error;

#[cfg(test)]
mod tests_jsx;

pub use codegen::{
    compile_module_esm, compile_project_esm, compile_project_with_jsx,
    compile_project_with_jsx_and_source_map, compile_with_jsx, EmittedJsModule, ImportRewrite,
    JsBundle, DEFAULT_JSX_IMPORT_SOURCE,
};
pub use error::CompileError;

/// Compile a single program to a bundle-style JS string. JSX lowers to `h` / `Fragment`; in bundle
/// mode those resolve against the merged scope (merge the `lattish` runtime for hooks and DOM). The
/// per-module ESM paths ([`compile_module_esm`] / [`compile_project_esm`]) auto-import the runtime
/// for JSX modules that don't import it themselves (issue #291).
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
