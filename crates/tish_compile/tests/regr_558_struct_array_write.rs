//! tishlang/tish#558 — a typed `Vec<struct>` element field write (`arr[i].field = v`) must lower to
//! a NATIVE store into the Vec element, not a `set_prop` on a throwaway `Value::object_from_pairs`
//! copy of the element (which silently discards the write). Regressed the module-level array and the
//! local array; both are covered here.
use std::path::PathBuf;
use tishlang_compile::compile_project_full;

fn compile(rel: &str) -> String {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let path = manifest.join("../..").join(rel).canonicalize().unwrap();
    compile_project_full(&path, path.parent(), &[], true).unwrap().0
}

#[test]
fn struct_array_write_is_a_native_store_not_a_discarded_copy() {
    let rust = compile("crates/tish_compile/tests/regression/struct_array_write.tish");

    // The write must NOT build a throwaway object and set_prop on it — that mutates a copy and
    // discards the store. This exact shape is what the bug emitted.
    assert!(
        !rust.contains("set_prop(&(Value::object_from_pairs"),
        "#558: `arr[i].field = v` must not set_prop on a throwaway object_from_pairs copy\n{rust}"
    );

    // It must instead be a direct field store into the Vec element (module-level `arr` here).
    assert!(
        rust.contains("arr[(0_f64) as usize].x ="),
        "#558: expected a native `arr[(0_f64) as usize].x = …` store into the Vec element\n{rust}"
    );
    // …and the local array `es` inside the function.
    assert!(
        rust.contains("es[(0_f64) as usize].x ="),
        "#558: expected a native `es[(0_f64) as usize].x = …` store into the local Vec element\n{rust}"
    );
}
