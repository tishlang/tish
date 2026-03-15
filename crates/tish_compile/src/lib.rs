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
    detect_cycles, extract_native_import_features, has_external_native_imports, has_native_imports,
    is_builtin_native_spec, merge_modules, resolve_native_modules, resolve_project,
    ResolvedNativeModule,
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

    #[test]
    fn loop_var_decl_clone_outer_var() {
        let src = r#"
let outerVar = 42
for (let i = 0; i < 5; i = i + 1) {
    let x = outerVar
}
"#;
        let program = parse(src).unwrap();
        let rust = compile(&program).unwrap();
        assert!(
            rust.contains("(outerVar).clone()"),
            "expected outerVar to be cloned in loop body"
        );
    }

    #[test]
    fn loop_var_decl_clone_via_project_full() {
        let manifest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let bench = manifest.join("../../tests/core/benchmark_granular.tish").canonicalize().unwrap();
        // Use same default features as tish CLI (http, fs, process, regex)
        let features = ["http", "fs", "process", "regex"]
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>();
        let (rust, _) = compile_project_full(&bench, bench.parent(), &features, true).unwrap();
        assert!(
            rust.contains("(outerVar).clone()"),
            "expected outerVar to be cloned in benchmark_granular loop"
        );
    }
}
