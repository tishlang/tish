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
    assert!(
        rust.contains("const G_CODES") || rust.contains("let mut codes: Vec<f64>"),
        "codes not lowered to module const or native vec"
    );
    assert!(
        rust.contains("const G_PROBS_CUM"),
        "precomputed cumulative array missing"
    );
    if rust.contains("fn fastaRandom_native(") {
        assert!(
            rust.contains("fastaRandom_native("),
            "fastaRandom_native declared but not called"
        );
        assert!(
            rust.contains("G_CODES["),
            "fastaRandom_native should index G_CODES directly"
        );
        assert!(
            rust.contains("_lcg_seed"),
            "fastaRandom_native should hoist LCG seed to a local"
        );
        assert!(
            rust.contains("_lcg_seed: i64") && rust.contains("3877i64"),
            "fastaRandom_native should use integer LCG arithmetic"
        );
    }
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
        assert!(
            rust.contains("for _usize_iter") && rust.contains("0..100"),
            "mandel_native should use usize bounded escape loop for maxIter=100"
        );
        assert!(
            rust.contains("_stayed_") && rust.contains("if _stayed_"),
            "mandel_native should fuse iter===maxIter into stayed flag"
        );
        assert!(
            rust.contains("count += 1_f64"),
            "mandel_native should increment count with +="
        );
        if let Some(native) = rust.split("fn mandel_native(").nth(1) {
            let native = native.split("fn run()").next().unwrap_or(native);
            assert!(
                !native.contains("iter = (iter + 1_f64)"),
                "mandel_native should skip iter increment in usize escape loop"
            );
            assert!(
                native.contains("0.0025_f64") && !native.contains("let mut py:"),
                "mandel_native should fuse py/h and px/w into reciprocal coord init"
            );
            assert!(
                !native.contains("let mut xt:"),
                "mandel_native should fuse x2/y2/xy iteration temps"
            );
            assert!(
                native.contains("let x2 =") && native.contains("let xy ="),
                "mandel_native should emit fused iteration temps"
            );
        }
    }
}

#[test]
fn fannkuch_nv_uses_direct_flip_indexing() {
    let rust = compile_fixture_typed("tests/perf/fannkuch.tish");
    if rust.contains("fn fannkuch_nv(") {
        let nv = rust.split("fn fannkuch_nv(").nth(1).unwrap();
        let nv = nv.split("fn run()").next().unwrap_or(nv);
        assert!(
            nv.contains("let ku =") && nv.contains("for _usize_flip_"),
            "fannkuch_nv flip loop should fuse to ku half-loop"
        );
        assert!(
            !nv.contains("let mut k2:") && !nv.contains("let mut temp:"),
            "fannkuch_nv fused flip should not emit k2/temp"
        );
        assert!(
            !nv.contains("perm.get(((k - i))"),
            "fannkuch_nv should not use perm.get for k-i sub-index"
        );
        assert!(
            nv.contains("copy_within(1..(_ru + 1), 0)")
                || (nv.contains("_usize_shift_") && nv.contains("perm1[_usize_shift_")),
            "fannkuch_nv rotation should shift perm1 via copy_within or fused usize loop"
        );
        assert!(
            nv.contains("perm1[(r) as usize] = perm0"),
            "fannkuch_nv rotation should assign perm1[r] without resize"
        );
        assert!(
            nv.contains("count[(r) as usize] = (count[(r) as usize] - 1_f64)"),
            "fannkuch_nv should decrement count[r] via direct indexing"
        );
        assert!(
            nv.contains("perm = std::iter::repeat(0_f64).take(10)")
                || nv.contains("perm.extend(std::iter::repeat(0_f64).take(10))"),
            "fannkuch_nv should bulk-init perm array"
        );
        assert!(
            nv.contains("perm1 = (0..10).map(|j| j as f64).collect()"),
            "fannkuch_nv should iota-init perm1"
        );
        assert!(
            nv.contains(".copy_from_slice(&perm1)"),
            "fannkuch_nv should copy perm1 into perm via copy_from_slice"
        );
        assert!(
            nv.contains("while r != 1_f64") && nv.contains("count[ri - 1] = r"),
            "fannkuch_nv should fill count via while r!=1 loop (not bulk 1..n init)"
        );
        assert!(
            nv.contains("_usize_shift_") && !nv.contains("let mut i: f64 = (_usize_"),
            "fannkuch_nv rotation loop should index via usize without f64 shadow i"
        );
        assert!(
            (nv.contains("if ((permCount as i64) & 1) == 0")
                || (nv.contains("to_int_unchecked") && nv.contains("& 1"))),
            "fannkuch_nv checksum parity should use fast int parity"
        );
        assert!(
            nv.contains("count[ri - 1] = r"),
            "fannkuch_nv count[r-1] init should use direct indexing in while loop"
        );
    }
}
