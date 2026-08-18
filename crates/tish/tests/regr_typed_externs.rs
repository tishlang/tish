//! Typed externs (perf lever): a native `cargo:` import whose typed signature is shipped in the
//! crate's `tish.d.tish` (auto-loaded from beside the crate source) must lower a matching call to a
//! DIRECT `<crate>::<name>_typed(..)` — no `Value` boxing of the args, no `value_call` dispatch —
//! while a call with no typed signature keeps the boxed namespace path. Uses the in-repo
//! `tests/fixtures/cargo_example_project/` fixture: `demo_shim` exports `greet` (untyped, stays boxed)
//! and `add` (declared `add(a: i32, b: i32): i32` in `crates/demo-shim/tish.d.tish`).
//!
//! Covers the three branch pieces with no other coverage: the `tish.d.tish` auto-load
//! (`compile_project_full_emit`), `collect_extern_fns` pairing, and the direct-call emission.

use std::path::PathBuf;

use tishlang_compile::compile_project_full;

fn emit_fixture() -> String {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let input_path = manifest_dir
        .join("tests/fixtures/cargo_example_project/src/main.tish")
        .canonicalize()
        .expect("cargo_example_project test fixture");
    let project_root = input_path.parent().map(|p| {
        if p.file_name().and_then(|n| n.to_str()) == Some("src") {
            p.parent().unwrap_or(p)
        } else {
            p
        }
    });
    compile_project_full(&input_path, project_root, &[], true)
        .expect("compile_project_full on the cargo example fixture")
        .0
}

#[test]
fn declared_extern_call_lowers_to_a_direct_typed_call() {
    let rust = emit_fixture();
    // The `add(2, 3)` call (typed via the auto-loaded tish.d.tish) must be a direct call into the
    // crate's `add_typed`, not a boxed dispatch through the namespace object.
    assert!(
        rust.contains("add_typed"),
        "typed extern `add` must lower to a direct `demo_shim::add_typed(..)` call; \
         the auto-loaded tish.d.tish declaration was not honored.\n{rust}"
    );
    // The direct call targets the backing crate.
    assert!(
        rust.contains("demo_shim :: add_typed") || rust.contains("demo_shim::add_typed"),
        "the direct typed-extern call must target the `demo_shim` crate.\n{rust}"
    );
}

#[test]
fn untyped_extern_call_stays_boxed() {
    let rust = emit_fixture();
    // `greet` has no typed signature (no tish.d.tish entry), so it must NOT gain a `_typed` call —
    // it keeps the boxed namespace/`value_call` path. Guards against collect_extern_fns over-matching.
    assert!(
        !rust.contains("greet_typed"),
        "untyped extern `greet` must stay boxed (no `greet_typed` direct call).\n{rust}"
    );
}
