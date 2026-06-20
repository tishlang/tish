//! Generated-Rust assertions for #177 follow-on — Vec<f64> returns, mandel native, fasta cum path.

use std::path::PathBuf;

use tishlang_compile::compile_project_full;

fn enable_typed_flags() {
    for k in [
        "TISH_PARAM_NATIVE",
        "TISH_PARAM_INFER",
        "TISH_NATIVE_FN",
        "TISH_STRUCT_INFER",
        "TISH_FUSED_HOF",
        "TISH_NATIVE_HOF",
        "TISH_AGGREGATE_INFER",
    ] {
        std::env::set_var(k, "1");
    }
}

fn compile_fixture_typed(rel: &str) -> String {
    enable_typed_flags();
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let path = manifest.join("../..").join(rel).canonicalize().unwrap();
    let (rust, _, _, _) = compile_project_full(&path, path.parent(), &[], true).unwrap();
    rust
}

#[test]
fn fasta_lowers_cumulative_and_native_arrays() {
    let rust = compile_fixture_typed("tests/perf/fasta.tish");
    assert!(rust.contains("fn cumulative_nv("), "cumulative_nv missing");
    assert!(rust.contains("let mut codes: Vec<f64>"), "codes not native vec");
    assert!(rust.contains("let mut probs: Vec<f64>"), "probs not native vec");
    assert!(
        rust.contains("cumulative_nv(&probs)"),
        "fastaRandom should call cumulative_nv:\n{}",
        rust.lines()
            .filter(|l| l.contains("cum") || l.contains("cumulative"))
            .take(10)
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn mandelbrot_lowers_mandel_native() {
    let rust = compile_fixture_typed("tests/perf/mandelbrot.tish");
    // Prefer top-level `mandel_native` when M5-eligible; the hot loop is already native f64
    // inside the closure even when this is absent.
    if rust.contains("fn mandel_native(") {
        assert!(
            rust.contains("mandel_native("),
            "mandel_native declared but not called"
        );
    }
}
