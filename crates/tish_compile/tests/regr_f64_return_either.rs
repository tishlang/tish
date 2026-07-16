//! tishlang/tish#485 — a sync fn returning one of two params must not have a param native-promoted
//! to f64 when called with args that may be non-numeric (string|null). No fn here does arithmetic,
//! so NO param should be f64-unboxed.
use std::path::PathBuf;
use tishlang_compile::compile_project_full;

fn compile(rel: &str) -> String {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let path = manifest.join("../..").join(rel).canonicalize().unwrap();
    let (rust, _, _, _) = compile_project_full(&path, path.parent(), &[], true).unwrap();
    rust
}

#[test]
fn sync_return_either_param_no_f64_unbox() {
    let rust = compile("tests/regression/f64_return_either_param.tish");
    std::fs::write(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/dump_485.rs"),
        &rust,
    ).ok();
    let offending: Vec<&str> = rust.lines().filter(|l| l.contains("expected number")).collect();
    assert!(
        offending.is_empty(),
        "tishlang/tish#485: no fn here does arithmetic, so no param should be f64-unboxed, but got:\n{}",
        offending.join("\n")
    );
}
