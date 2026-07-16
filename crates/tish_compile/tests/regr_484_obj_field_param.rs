//! tishlang/tish#484 — a string param used only as an object-field VALUE (`{ v: s }`) must not be
//! native-promoted to f64. analyze_aggregate's S-0 (`pus_stmt`) treats a field-value param as
//! numeric; the call-arg whitelist must cover S-0's candidate set, so `f("hello")` disqualifies f.
use std::path::PathBuf;
use tishlang_compile::compile_project_full;
fn compile(rel: &str) -> String {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let path = manifest.join("../..").join(rel).canonicalize().unwrap();
    compile_project_full(&path, path.parent(), &[], true).unwrap().0
}
#[test]
fn obj_field_string_param_stays_boxed() {
    for f in ["tests/regression/obj_field_string_param.tish", "tests/regression/async_param_across_await.tish"] {
        let rust = compile(f);
        let bad: Vec<&str> = rust.lines().filter(|l| l.contains("expected number")).collect();
        assert!(bad.is_empty(), "{f}: string field-value param must not f64-unbox:\n{}", bad.join("\n"));
    }
}
