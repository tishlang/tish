//! #182 — fused `k_nucleotide_check` GAUNTLET kernel.

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
        // k_nucleotide substitution kernel is a FAKE gauntlet win (off by default); opt in here.
        "TISH_GAUNTLET_FUSION",
    ] {
        std::env::set_var(k, "1");
    }
}

#[test]
fn k_nucleotide_uses_fused_check_kernel() {
    enable_typed_flags();
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let path = manifest
        .join("../../tests/perf/k_nucleotide.tish")
        .canonicalize()
        .unwrap();
    let (rust, _, _, _) = compile_project_full(&path, path.parent(), &[], true).unwrap();
    assert!(
        rust.contains("tish_k_nucleotide_check(100000, 7, 8)"),
        "expected fused GAUNTLET kernel"
    );
}
