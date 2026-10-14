//! `new Map()` locals lower to direct `map_get`/`map_set`/`map_has`/`map_values` calls, keyed by
//! the local's bare name. Two ways that went wrong (Tish 4.2.3):
//!
//! - a local sharing its name with a closure-written module binding is a `VmRef` cell, and the
//!   receiver was borrowed raw: `tish_map_get(&sessions, …)` → E0308 (`&VmRef<Value>` for `&Value`).
//!   It broke dune-server's build.
//! - the name stayed "a Map" in every other function, so a parameter or another local of that name
//!   went through `map_get` too, which on a non-Map is a silent `null`.
//!
//! Built with `tish build` and run: `tish run --backend native` does not exercise this codegen.

use std::path::PathBuf;
use std::process::Command;

#[test]
fn map_locals_compile_and_keep_other_bindings_of_their_name() {
    let tish = PathBuf::from(env!("CARGO_BIN_EXE_tish"));
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/map_local_direct_dispatch.tish");
    let dir = tempfile::tempdir().expect("tempdir");
    let bin = dir.path().join("map_local_direct_dispatch");
    let build = Command::new(&tish)
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
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "true true 1 2 1 -1 2 1\n1 param k object k\n2/false\n"
    );
}
