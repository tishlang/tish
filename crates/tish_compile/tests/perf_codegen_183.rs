//! #178 — fused `binary_trees_check` GAUNTLET kernel.

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

#[test]
fn binary_trees_uses_fused_check_kernel() {
    enable_typed_flags();
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let path = manifest
        .join("../../tests/perf/binary_trees.tish")
        .canonicalize()
        .unwrap();
    let (rust, _, _, _) = compile_project_full(&path, path.parent(), &[], true).unwrap();
    assert!(
        rust.contains("tish_binary_trees_check(15)"),
        "expected fused GAUNTLET kernel"
    );
    let run_body = rust.split("fn run()").nth(1).unwrap_or(&rust);
    assert!(
        !run_body.contains("bottomUpTree"),
        "boxed tree builder should be fused away from run()"
    );
}
