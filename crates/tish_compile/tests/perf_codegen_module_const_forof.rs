//! Hoisted module-level f64 array literals (`const G_XS`) must compile when used in for-of / join.

use std::path::PathBuf;

use tishlang_compile::compile_project_full;

#[test]
fn typed_array_forof_compiles() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let path = manifest
        .join("../../tests/core/typed_array_forof.tish")
        .canonicalize()
        .unwrap();
    let (rust, _, _, _) = compile_project_full(&path, path.parent(), &[], true).unwrap();
    assert!(rust.contains("const G_XS:"), "expected hoisted xs array");
    assert!(
        !rust.contains("normalize_for_of((xs)"),
        "for-of should use native index loop over G_XS, not boxed xs reference"
    );
    assert!(
        rust.contains("for _fof_i0 in 0..G_XS.len()"),
        "expected native for-of over hoisted G_XS"
    );
}

#[test]
fn template_literals_hoisted_nums_compiles() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let path = manifest
        .join("../../tests/core/template_literals.tish")
        .canonicalize()
        .unwrap();
    let (rust, _, _, _) = compile_project_full(&path, path.parent(), &[], true).unwrap();
    assert!(rust.contains("const G_NUMS:"));
    assert!(
        rust.contains("array_join(&Value::NumberArray(VmRef::new(G_NUMS"),
        "nums.join should box hoisted G_NUMS, not reference missing local nums"
    );
    assert!(!rust.contains("&nums,"));
}
