//! Native compiler backend for Tish.
//!
//! Emits Rust source that links to tish_runtime.

mod codegen;
mod resolve;
mod types;

pub use codegen::{
    compile, compile_project, compile_project_full, compile_with_features,
    compile_with_native_modules, compile_with_project_root,
};
pub use codegen::CompileError;
pub use resolve::{
    detect_cycles, extract_native_import_features, has_native_imports, merge_modules,
    resolve_native_modules, resolve_project, ResolvedNativeModule,
};
pub use types::{RustType, TypeContext};

#[cfg(test)]
mod tests {
    use super::*;
    use tish_parser::parse;

    #[test]
    fn typed_assign_conversion() {
        let src = r#"
fn sum(...args: number[]): number {
    let total: number = 0
    for (let n of args) { total = total + n }
    return total
}
"#;
        let program = parse(src).unwrap();
        let rust = compile(&program).unwrap();
        assert!(rust.contains("match &_v { Value::Number(n) => *n"), "expected typed assign conversion");
    }
}
