//! A function run by `Promise.spawn` runs on its own OS thread in a native build. It must see the
//! module's top-level functions and globals, not a fresh thread-local copy of them: module statics
//! and `thread_local` numeric globals are only safe when no Tish code can run off the main thread.
//! 4.2.2 enabled them for `--feature http` programs (anything but `ws`), so a spawned closure
//! called `null` ("Call of non-function: null") and a numeric global's write was lost.

use std::path::PathBuf;
use std::process::Command;

fn run_native(features: Option<&str>) -> String {
    let tish = PathBuf::from(env!("CARGO_BIN_EXE_tish"));
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/core/promise_spawn_module_globals.tish");
    let mut args = vec!["run", "--backend", "native"];
    if let Some(f) = features {
        args.extend(["--feature", f]);
    }
    args.push(path.to_str().unwrap());
    let out = Command::new(&tish).args(&args).output().expect("spawn tish run");
    assert!(
        out.status.success(),
        "tish run {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

const EXPECTED: &str = "11\n43 43\n7 done job 3\n";

#[test]
fn spawned_closure_sees_module_globals_with_http() {
    assert_eq!(run_native(Some("http")), EXPECTED);
}

#[test]
fn spawned_closure_sees_module_globals_full_build() {
    assert_eq!(run_native(None), EXPECTED);
}
