//! `collection_instance_locals` keeps a `new Map()` / `new Set()` local's `keys`/`values`/`entries`
//! off the array fast path. It was keyed by bare name across the whole program, so an ARRAY of the
//! same name in another function also left the fast path and called the method as a bound method,
//! which an array does not have. It is now scoped per function / closure body, as #772 did for the
//! Map-local direct dispatch.
//!
//! Built with `tish build` and run: `tish run --backend native` does not exercise this codegen.

use std::path::PathBuf;
use std::process::Command;

#[test]
fn collection_locals_do_not_capture_same_named_arrays() {
    let tish = PathBuf::from(env!("CARGO_BIN_EXE_tish"));
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/collection_local_scope.tish");
    let dir = tempfile::tempdir().expect("tempdir");
    let bin = dir.path().join("collection_local_scope");
    let build = Command::new(&tish)
        .env("TISH_FAST_NATIVE_BUILD", "1")
        .args(["build", "--target", "native", "-o"])
        .arg(&bin)
        .arg(&src)
        .output()
        .expect("spawn tish build");
    assert!(
        build.status.success(),
        "native build failed:\n{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let out = Command::new(&bin).output().expect("run the built binary");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "a,b k=1 2\n0,1,2 10,20,0:10,1:20 p,q\n"
    );
}
