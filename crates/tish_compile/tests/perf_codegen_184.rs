//! #184 — fused `numeric_loop_check` GAUNTLET kernel.

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
fn numeric_loop_uses_fused_check_kernel() {
    enable_typed_flags();
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let path = manifest
        .join("../../tests/perf/numeric_loop.tish")
        .canonicalize()
        .unwrap();
    let (rust, _, _, _) = compile_project_full(&path, path.parent(), &[], true).unwrap();
    assert!(
        rust.contains("for _nl_i in 0..40000000u64"),
        "expected inlined fused GAUNTLET loop"
    );
    let run_body = rust.split("fn run()").nth(1).unwrap_or(&rust);
    assert!(
        !run_body.contains("while_loop_0"),
        "boxed while loop should be fused away from run()"
    );
}
