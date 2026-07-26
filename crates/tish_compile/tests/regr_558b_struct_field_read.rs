//! tishlang/tish#558 residual — a struct-field read from a Vec element in a value/return position must
//! clone a non-Copy field instead of moving it out of the borrowed element (E0507).
use std::path::PathBuf;
use tishlang_compile::compile_project_full;

fn compile(rel: &str) -> String {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let path = manifest.join("../..").join(rel).canonicalize().unwrap();
    compile_project_full(&path, path.parent(), &[], true).unwrap().0
}

#[test]
fn struct_field_read_from_vec_element_clones_non_copy() {
    let rust = compile("crates/tish_compile/tests/regression/struct_field_return_read.tish");
    // Off-GBA the `: i32` field is a boxed `Value` (non-Copy); the read must clone it out of the
    // borrowed `es[0]`, not move it.
    assert!(
        rust.contains("es[(0_f64) as usize]).x.clone()"),
        "#558 residual: `es[0].x` read must clone the non-Copy field out of the borrowed element\n{rust}"
    );
}
