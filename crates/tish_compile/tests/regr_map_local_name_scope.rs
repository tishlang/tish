//! #181's direct Map dispatch is keyed by a local's bare name. It is scoped to the function that
//! declares the Map: a parameter or another local of the same name elsewhere keeps its own
//! meaning, and a Map local that lives in a `VmRef` cell is read out of the cell first.

use std::path::PathBuf;

use tishlang_compile::compile_project_full;

fn rust() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../tish/tests/fixtures/map_local_direct_dispatch.tish")
        .canonicalize()
        .unwrap();
    compile_project_full(&path, path.parent(), &[], true)
        .unwrap()
        .0
}

#[test]
fn map_dispatch_stays_in_the_function_that_declares_the_map() {
    let rust = rust();
    // `counts` and `mapLocal`'s `m` are Maps where they are used: the fast path stays.
    assert!(
        rust.contains("tish_map_get(&counts,"),
        "counts lost the direct path"
    );
    assert!(
        rust.contains("tish_map_set(&counts,"),
        "counts lost the direct path"
    );
    // `m` is ALSO a parameter of `param` and an object in `objectLocal`: only `mapLocal`'s one
    // `set` and one `get` may go direct.
    assert_eq!(
        rust.matches("tish_map_get(&m,").count(),
        1,
        "a non-Map `m` went through map_get"
    );
    assert_eq!(rust.matches("tish_map_set(&m,").count(), 1);
}

#[test]
fn a_map_in_a_cell_is_read_out_of_the_cell() {
    let rust = rust();
    // `makeRegistry`'s `sessions` shares its name with a closure-written module binding, so it is a
    // `VmRef` cell. Borrowing the cell itself (`&sessions`) was E0308 in 4.2.3.
    assert!(
        !rust.contains("tish_map_get(&sessions,"),
        "borrowed the VmRef cell as the Map"
    );
    assert!(
        !rust.contains("tish_map_set(&sessions,"),
        "borrowed the VmRef cell as the Map"
    );
    assert!(rust.contains("tish_map_get(&(tishlang_runtime::vm_read(&sessions)"));
}
