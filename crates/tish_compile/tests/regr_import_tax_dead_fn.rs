//! Import tax: unreachable top-level FunDecls must not become `Value::native` heap closures.
//!
//! Importing a package used to materialise every function in it at module init (~151 bytes each),
//! whether called or not. Reachability from module-level code (and RustLib exports) decides which
//! decls still need a boxed binding.

use std::path::PathBuf;

use tishlang_compile::compile_project_full;

fn compile_src(stem: &str, src: &str) -> String {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/regr_import_tax");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{stem}.tish"));
    std::fs::write(&path, src).unwrap();
    let path = path.canonicalize().unwrap();
    let (rust, _, _, _) = compile_project_full(&path, path.parent(), &[], true).unwrap();
    rust
}

#[test]
fn unused_top_level_fn_is_not_a_heap_closure() {
    let rust = compile_src(
        "dead_fn",
        r#"
function deadHelper(x) { return x + 1 }
function used(x) { return x * 2 }
console.log(used(21))
"#,
    );
    assert!(
        rust.contains("let used = {") || rust.contains("fn used_native") || rust.contains("fn used_bfn"),
        "used() must still be emitted (boxed or native): {rust}"
    );
    assert!(
        !rust.contains("let deadHelper = {"),
        "deadHelper must not become a Value::native binding:\n{rust}"
    );
    assert!(
        !rust.contains("deadHelper_cell"),
        "deadHelper must not get a VmRef cell:\n{rust}"
    );
}

#[test]
fn transitive_callee_of_used_fn_is_kept() {
    let rust = compile_src(
        "transitive",
        r#"
function leaf(x) { return x + 1 }
function mid(x) { return leaf(x) * 2 }
function unused(x) { return leaf(x) }
console.log(mid(3))
"#,
    );
    assert!(
        rust.contains("let leaf = {") || rust.contains("fn leaf_bfn") || rust.contains("fn leaf_native"),
        "leaf must stay — mid calls it:\n{rust}"
    );
    assert!(
        rust.contains("let mid = {") || rust.contains("fn mid_bfn") || rust.contains("fn mid_native"),
        "mid must stay:\n{rust}"
    );
    assert!(
        !rust.contains("let unused = {"),
        "unused must be omitted:\n{rust}"
    );
    assert!(
        !rust.contains("let leaf = {") || rust.contains("fn leaf_bfn"),
        "call-only leaf should prefer free fn over Value::native when hoist succeeds"
    );
}

#[test]
fn call_only_fns_emit_as_free_bfn_without_heap_binding() {
    let rust = compile_src(
        "call_only_bfn",
        r#"
function leaf(x) { return x + 1 }
function mid(x) { return leaf(x) * 2 }
console.log(mid(3))
"#,
    );
    // leaf may lower as M5 `leaf_native` (typed) or residual `leaf_bfn` (boxed ABI free fn).
    assert!(
        rust.contains("fn leaf_bfn(") || rust.contains("fn leaf_native("),
        "leaf must be a free fn:\n{rust}"
    );
    assert!(
        rust.contains("fn mid_bfn(args: &[Value]) -> Value") || rust.contains("fn mid_native("),
        "mid must be a free fn:\n{rust}"
    );
    assert!(
        !rust.contains("let leaf = {") && !rust.contains("leaf_cell"),
        "leaf must not allocate Value::native:\n{rust}"
    );
    assert!(
        !rust.contains("let mid = {") && !rust.contains("mid_cell"),
        "mid must not allocate Value::native:\n{rust}"
    );
    assert!(
        rust.contains("mid_bfn(") || rust.contains("mid_native("),
        "call sites must route to a free fn:\n{rust}"
    );
}

#[test]
fn value_used_fn_is_kept_even_if_never_called() {
    let rust = compile_src(
        "value_use",
        r#"
function handler(x) { return x }
let table = [handler]
console.log(table.length)
"#,
    );
    assert!(
        rust.contains("let handler = {"),
        "handler stored as a value must keep its Value::native binding:\n{rust}"
    );
}
