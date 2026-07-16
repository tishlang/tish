//! tishlang/tish — a typed Option (`string?`) captured by a long-lived FnMut closure (a `serve()`
//! handler, called per request) must not be MOVED when boxed to a Value for a `=== null` comparison.
//! `to_value_expr(Option)` must match by REFERENCE (`match &(…)`), like the Vec arm's `.iter()`.
use std::path::PathBuf;
use tishlang_compile::compile_project_full;
fn compile(rel: &str) -> String {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let path = manifest.join("../..").join(rel).canonicalize().unwrap();
    compile_project_full(&path, path.parent(), &[], true).unwrap().0
}
#[test]
fn option_boxing_borrows_not_moves() {
    let rust = compile("tests/regression/serve_closure_captures_typed.tish");
    assert!(
        rust.contains("match &(sToken)"),
        "Option→Value boxing must match by reference so the source isn't moved"
    );
    assert!(
        !rust.contains("match sToken { Some(v)"),
        "Option→Value boxing must not match by value (moves the closure-captured Option)"
    );
}
