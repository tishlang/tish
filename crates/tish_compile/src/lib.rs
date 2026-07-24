//! Native compiler backend for Tish.
//!
//! Emits Rust source that links to tishlang_runtime.

mod check;
mod codegen;
mod infer;
mod platform_resolve;
mod resolve;
mod schemes;
mod types;

pub use schemes::{set_active as set_scheme_registry, SchemeRegistry};

pub use check::{check_program, TypeDiagnostic};

/// The native typed-codegen optimizations — numeric param inference, struct/aggregate inference,
/// native (monomorphic) free fns, native-vec params, recursive-struct arena lowering, fused/native
/// higher-order fns, and native `number[]` params — are **ON BY DEFAULT**. There are no per-pass
/// flags anymore: that per-flag gating caused repeated "did I set all of them?" drift between the
/// gauntlet, manual builds, and CI. The single escape hatch `TISH_NATIVE_OPT=0` turns the whole
/// stack off — used only by the gauntlet's boxed A/B baseline and to bisect a suspected miscompile.
pub(crate) fn native_opts_enabled() -> bool {
    std::env::var("TISH_NATIVE_OPT").map(|v| v != "0").unwrap_or(true)
}

/// #381 — the native recursion guard: rotation copies for self/mutually-recursive typed fns plus a
/// depth guard at boxed user-fn closure entry, so unbounded recursion raises a catchable
/// `RangeError` instead of overflowing the stack (an uncatchable process abort). **Default ON**;
/// `TISH_NATIVE_RECUR_GUARD=0` turns it off — mirrors the VM JIT tier's `TISH_JIT_RECUR_GUARD`
/// (for bisection and code-size-sensitive builds; unbounded recursion then aborts again, as before).
pub(crate) fn native_recur_guard_enabled() -> bool {
    std::env::var("TISH_NATIVE_RECUR_GUARD").map(|v| v != "0").unwrap_or(true)
}

/// How generated Rust is linked (desktop binary vs embedded iOS staticlib vs GBA ROM).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NativeEmitMode {
    #[default]
    DesktopBin,
    /// `[lib] crate-type = ["staticlib"]` — no `fn main()`, host calls `tish_ios_launch`.
    EmbeddedLib,
    /// Game Boy Advance ROM: `#![no_std]`, `#[agb::entry] fn agb_main(gba)`, links the
    /// `tishlang_runtime_gba` facade. Numbers/async lowering diverge; see codegen `emit_program`.
    Gba,
}

pub use codegen::CompileError;
pub use codegen::{
    compile, compile_project, compile_project_full, compile_project_full_emit,
    compile_with_features, compile_with_native_modules, compile_with_native_modules_emit,
    compile_with_project_root,
};
pub use platform_resolve::{
    apply_resolve_env, parse_platform, parse_surface, platform_suffixes, platform_virtual_keys,
    resolve_context, resolve_id_public, resolve_with_platform, set_resolve_context, Platform,
    ResolveContext, Surface,
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
        // Typed rest-param `...args: number[]` lowers to a native `Vec<f64>` (M3), so the ForOf
        // element `n` is `f64`, `total = total + n` stays native, and `total` is NOT demoted — the
        // whole reduction compiles to native f64 with the return wrapping `total` back to `Value`.
        let src = r#"
fn sum(...args: number[]): number {
    let total: number = 0
    for (let n of args) { total = total + n }
    return total
}
"#;
        let program = parse(src).unwrap();
        let rust = compile(&program).unwrap();
        assert!(
            rust.contains("let mut total: f64"),
            "typed rest-param `Vec<f64>` keeps `total` native f64 (no demotion)"
        );
        assert!(
            rust.contains("Value::Number(total)"),
            "f64 total is wrapped back to Value at the return boundary"
        );

        // When every reassignment is provably numeric, the `number` local stays native `f64` and
        // is wrapped back to `Value` only at the return boundary.
        let src_native = r#"
fn count(): number {
    let total: number = 0
    for (let i: number = 0; i < 10; i = i + 1) { total = total + i }
    return total
}
"#;
        let program = parse(src_native).unwrap();
        let rust = compile(&program).unwrap();
        assert!(
            rust.contains("let mut total: f64"),
            "numeric-only reassignment keeps `total` native f64"
        );
        assert!(
            rust.contains("Value::Number(total)"),
            "f64 total is wrapped back to Value at the return boundary"
        );
    }

    #[test]
    fn loop_var_decl_clone_outer_var() {
        // outerVar = 42 is inferred as f64. Per #313, a top-level numeric used ONLY at top level
        // (never inside a function/closure body) is a native `run()`-local `let mut _: f64`, NOT a
        // `thread_local Cell<f64>` — the Cell is reserved for globals a function reads across calls
        // (e.g. fasta's `seed`). The read `let x = outerVar` loads it as a Copy f64.
        let src = r#"
let outerVar = 42
for (let i = 0; i < 5; i = i + 1) {
    let x = outerVar
}
"#;
        let program = parse(src).unwrap();
        let rust = compile(&program).unwrap();
        // outerVar is a native f64 local (no TLS Cell, since no function references it); x reads it.
        assert!(
            rust.contains("let mut outerVar: f64"),
            "expected outerVar as a native f64 local (#313)"
        );
        assert!(
            !rust.contains("G_OUTERVAR"),
            "outerVar must NOT be a thread_local Cell global — it is top-level-only (#313)"
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

    /// i32-loop-var lowering: an FNV-style integer/bitwise hash accumulator declared before a `for`
    /// and reassigned only by `>>> 0`/bitwise ops lives in an `i32` register across the loop — no
    /// per-op `to_int32(h)`↔`f64` round-trip — with a single `f64` excursion for the `h * C`
    /// multiply (which exceeds 2^53, so it must round in f64 before `>>> 0`, exactly as V8 does).
    #[test]
    fn fnv_accumulator_lowers_to_i32_register() {
        let src = r#"
let h = 2166136261
for (let i = 0; i < 100; i++) {
  h = h ^ (i & 255)
  h = (h * 16777619) >>> 0
  h = ((h << 13) | (h >>> 19)) >>> 0
}
let check = h >>> 0
console.log(check)
"#;
        let rust = compile(&parse(src).unwrap()).unwrap();
        // The accumulator is an i32 register, initialized via the u32 reinterpretation so the
        // > i32::MAX seed keeps its JS ToInt32 bit-pattern.
        assert!(
            rust.contains("let mut h: i32 = (2166136261u32) as i32;"),
            "expected `h` to be an i32 register seeded via u32 reinterpretation; got:\n{}",
            rust.lines().filter(|l| l.contains("h")).take(8).collect::<Vec<_>>().join("\n")
        );
        // The per-iteration `to_int32(h)` round-trips are gone: `h` is read straight from the
        // register inside the bitwise chain (no `to_int32(h)` substring referencing the accumulator).
        assert!(
            !rust.contains("to_int32(h)"),
            "expected NO per-op `to_int32(h)` round-trip on the i32 accumulator"
        );
        // The only f64 excursion is the multiply, lowered as an unchecked truncation of the
        // provably-finite product (`h as f64 * 16777619`).
        assert!(
            rust.contains(".to_int_unchecked::<i64>()") && rust.contains("16777619"),
            "expected the `h * 16777619` excursion to lower to an unchecked f64 truncation"
        );
    }
}

#[cfg(test)]
mod monomorphization_tests {
    use super::*;
    use tishlang_parser::parse;

    /// `Box<number>` monomorphizes to a synthetic concrete alias whose field is a native `f64`,
    /// not a boxed `Value` — generic structs participate in native lowering.
    #[test]
    fn generic_struct_is_native() {
        let src = "type Box<T> = { value: T }\nlet b: Box<number> = { value: 42 }\nconsole.log(b.value + 1)";
        let rust = compile(&parse(src).unwrap()).unwrap();
        // Box<number> must monomorphize to a struct with a native f64 field (not Value).
        assert!(rust.contains("value: f64"), "expected native f64 field; got:\n{}",
            rust.lines().filter(|l| l.contains("struct") || l.contains("value")).take(6).collect::<Vec<_>>().join("\n"));
    }
}
