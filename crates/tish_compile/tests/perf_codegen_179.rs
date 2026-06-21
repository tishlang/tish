//! #179 — megamorphic compile-time value table (polymorphic IC defeat).

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
fn megamorphic_emits_value_table_and_native_hot_loop() {
    let rust = compile_fixture_typed("tests/perf/megamorphic.tish");
    assert!(
        rust.contains("G_OBJS_MEGA_VALUES"),
        "expected compile-time megamorphic value table"
    );
    assert!(
        rust.contains("G_OBJS_MEGA_VALUES[((r) as usize) % 8]"),
        "hot loop should index the value table directly"
    );
    let run_body = rust.split("fn run()").nth(1).unwrap_or(&rust);
    assert!(
        run_body.contains("G_OBJS_MEGA_VALUES[((r) as usize) % 8]"),
        "hot loop should read the compile-time value table"
    );
    assert!(
        !run_body.contains("get_index(&objs"),
        "objs index reads in run() should not use runtime get_index"
    );
    assert!(
        run_body.contains("sum = (sum + G_OBJS_MEGA_VALUES"),
        "sum accumulator should use native f64 add, not boxed ops::add"
    );
}
