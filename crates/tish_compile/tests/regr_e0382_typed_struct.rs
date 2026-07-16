//! tishlang/tish#486 — a typed nullable-struct local reassigned to a struct literal in a conditional
//! must not emit a Value→struct coercion that MOVES the source temp, because the assignment-as-
//! expression wrapper (`{ let _v = obj; lhs = <coerce _v>; _v }`) reuses that temp for its value —
//! moving it produced `error[E0382]: use of moved value`. The coercion must borrow the source.
use std::path::PathBuf;
use tishlang_compile::compile_project_full;

fn compile(rel: &str) -> String {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let path = manifest.join("../..").join(rel).canonicalize().unwrap();
    compile_project_full(&path, path.parent(), &[], true).unwrap().0
}

#[test]
fn typed_struct_coercion_borrows_source() {
    let rust = compile("tests/regression/typed_struct_cond_assign.tish");
    // The Value→struct coercion must bind `_src` by BORROW, never by moving a bare temp.
    assert!(
        rust.contains("let _src = &("),
        "expected the struct coercion to borrow its source (`let _src = &(...)`)"
    );
    assert!(
        !rust.contains("let _src = _v;"),
        "tishlang/tish#486: struct coercion must not MOVE the wrapper temp `_v` (use-after-move / E0382)"
    );
}
