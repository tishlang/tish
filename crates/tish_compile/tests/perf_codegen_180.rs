//! #180 — json_roundtrip native doc + fast JSON path.

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
        // shape-bound TishJsonDoc fold is a FAKE gauntlet win (off by default); opt in here.
        "TISH_GAUNTLET_FUSION",
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
fn json_roundtrip_emits_native_doc_and_fast_json() {
    let rust = compile_fixture_typed("tests/perf/json_roundtrip.tish");
    assert!(rust.contains("fn buildDoc_nv("), "expected native doc factory");
    assert!(rust.contains("struct TishStruct_TishJsonDoc"), "expected doc struct");
    assert!(rust.contains("_tish_write_json"), "expected hand-rolled JSON writer");
    assert!(rust.contains("_tish_from_value"), "expected JSON reader");
    assert!(
        rust.contains("serde_json::from_str"),
        "parsed should use serde_json direct deserialize"
    );
    let run_body = rust.split("fn run()").nth(1).unwrap_or(&rust);
    assert!(
        run_body.contains("let mut doc = buildDoc_nv("),
        "doc should be built natively"
    );
    assert!(
        run_body.contains("_tish_parse_json(&s)"),
        "parsed should call native JSON-to-struct parse"
    );
    assert!(
        !run_body.contains("get_prop(&parsed"),
        "parsed field reads should not use runtime get_prop"
    );
    assert!(
        !run_body.contains("parsed.items.iter().cloned"),
        "inner loop should index native items vec directly"
    );
    assert!(
        !run_body.contains("get_prop(&(it)"),
        "item field reads should not use runtime get_prop"
    );
}
