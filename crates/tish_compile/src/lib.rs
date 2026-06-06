//! Native compiler backend for Tish.
//!
//! Emits Rust source that links to tishlang_runtime.

mod codegen;
mod infer;
mod resolve;
mod types;

/// How generated Rust is linked (desktop binary vs embedded iOS staticlib).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NativeEmitMode {
    #[default]
    DesktopBin,
    /// `[lib] crate-type = ["staticlib"]` — no `fn main()`, host calls `tish_ios_launch`.
    EmbeddedLib,
}

pub use codegen::CompileError;
pub use codegen::{
    compile, compile_project, compile_project_full, compile_project_full_emit,
    compile_with_features, compile_with_native_modules, compile_with_native_modules_emit,
    compile_with_project_root,
};
pub use resolve::{
    cargo_export_fn_name, compute_native_build_artifacts, detect_cycles, ensure_tish_canvas_module,
    export_name_to_rust_ident, extract_native_import_features, format_rust_dependencies_toml,
    generate_native_wrapper_rs, has_external_native_imports, has_native_imports,
    ffi_native_specs, infer_native_module_exports, is_builtin_native_spec, is_cargo_native_spec,
    is_ffi_native_spec, is_native_import,
    merge_modules, normalize_builtin_spec, program_uses_document, read_project_tish_config,
    resolve_bare_spec, resolve_native_modules, resolve_project, resolve_project_from_stdin,
    MergedProgram, NativeBuildArtifacts, NativeModuleInit, ResolvedNativeModule,
};
pub use types::{RustType, TypeContext};

#[cfg(test)]
mod tests {
    use super::*;
    use tishlang_parser::parse;

    #[test]
    fn typed_assign_conversion() {
        // With the inference pass and native emit, `total: number = 0` becomes f64.
        // Assignment `total = total + n` (where n comes from ForOf over a Value::Array)
        // emits a native f64 assignment that unboxes the Value result via from_value_expr.
        let src = r#"
fn sum(...args: number[]): number {
    let total: number = 0
    for (let n of args) { total = total + n }
    return total
}
"#;
        let program = parse(src).unwrap();
        let rust = compile(&program).unwrap();
        // total should be declared as f64
        assert!(rust.contains("let mut total: f64"), "expected total: f64");
        // The return value of run() should convert total back to Value
        assert!(
            rust.contains("Value::Number(total)"),
            "expected Value::Number(total) wrapping"
        );
    }

    #[test]
    fn loop_var_decl_clone_outer_var() {
        // With inference, outerVar = 42 gets inferred as f64. f64 is Copy, so no clone is
        // needed — direct assignment is correct. The test verifies compilation succeeds.
        let src = r#"
let outerVar = 42
for (let i = 0; i < 5; i = i + 1) {
    let x = outerVar
}
"#;
        let program = parse(src).unwrap();
        let rust = compile(&program).unwrap();
        // outerVar and x are f64 (inferred) — Copy assignment, no .clone() needed.
        assert!(
            rust.contains("let mut outerVar: f64"),
            "expected outerVar: f64"
        );
        assert!(rust.contains("let mut x: f64"), "expected x: f64");
    }

    #[test]
    fn new_expression_lowers_to_construct_on_native() {
        let src = "fn f() { return new Uint8Array(4) }";
        let program = parse(src).unwrap();
        let rust = compile(&program).unwrap();
        assert!(
            rust.contains("tish_construct"),
            "expected new to lower to tish_construct, got snippet missing it"
        );
    }

    /// User-defined constructor name: `new ClassName(...)` must compile natively (host `construct`)
    /// and is the same surface syntax as the JS target (`new` in emitted JavaScript).
    #[test]
    fn new_class_name_compiles_native_via_tish_construct() {
        let src = r#"
fn ClassName(x) {
    return x
}
fn factory() {
    return new ClassName(42)
}
"#;
        let program = parse(src).unwrap();
        let rust = compile(&program).unwrap();
        assert!(
            rust.contains("tish_construct"),
            "expected new ClassName to lower to tish_construct"
        );
        assert!(
            rust.contains("ClassName"),
            "expected emitted Rust to reference ClassName callable"
        );
    }

    /// `value_call` must take `&Value` to a **local** (`let _callee = (<expr>).clone(); … &_callee`):
    /// `&<temporary>` can dangle in release, and `let _callee = <ident>` would move globals like `Symbol`.
    #[test]
    fn native_emit_value_call_materializes_callee() {
        use std::path::PathBuf;
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let path = manifest.join("../../tests/core/symbol.tish").canonicalize().unwrap();
        let (rust, _, _, _) = compile_project_full(&path, path.parent(), &[], true).unwrap();
        assert!(
            rust.contains("let _callee = (tishlang_runtime::get_index"),
            "fixture should bracket-call via get_index with callee stored in a local"
        );
        assert!(
            !rust.contains("let _callee = &tishlang_runtime::get_index"),
            "expected callee materialization, found reference-to-temporary pattern"
        );
        assert!(
            rust.contains("tishlang_runtime::value_call"),
            "expected value_call via runtime re-export for nested Cargo builds"
        );
    }

    #[test]
    fn loop_var_decl_clone_via_project_full() {
        // With the inference pass, `let outerVar = 42` is inferred as f64 (Copy) — no clone needed.
        // This test verifies the full benchmark_granular project compiles and that outerVar
        // is emitted as the inferred f64 type rather than requiring a Value clone.
        let manifest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let bench = manifest
            .join("../../tests/core/benchmark_granular.tish")
            .canonicalize()
            .unwrap();
        // Use same default features as tish CLI (http, fs, process, regex)
        let features = ["http", "fs", "process", "regex"]
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>();
        let (rust, _, _, _) =
            compile_project_full(&bench, bench.parent(), &features, true).unwrap();
        // outerVar = 42 is inferred as f64; f64 is Copy so no .clone() is emitted.
        assert!(
            rust.contains("let mut outerVar: f64"),
            "expected outerVar to be inferred as f64 (Copy, no clone needed)"
        );
    }
}
