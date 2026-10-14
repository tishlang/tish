//! `Promise` is in scope for every feature set whose runtime links the promise machinery — `http`,
//! `ws`, `fs` (fs/promises) and `promise` — whether or not the program uses `await`. A program
//! without top-level `await` built with Dune's `--feature http,fs,process` failed to compile
//! ("cannot find value `Promise`"), and `tish run --feature fs` / `--feature ws` reported
//! "Undefined variable: Promise".
//!
//! The native cases build with `tish build` and run the binary: `tish run --backend native` runs
//! the bytecode VM, so it cannot catch a native compile failure.

use std::path::PathBuf;
use std::process::Command;

const EXPECTED: &str = "object\nfunction\nfunction\ntrue\n";

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/promise_without_await.tish")
}

fn build_and_run(features: &str) -> String {
    let tish = PathBuf::from(env!("CARGO_BIN_EXE_tish"));
    let dir = tempfile::tempdir().expect("tempdir");
    let out_bin = dir.path().join("promise_without_await");
    let out = Command::new(&tish)
        .env("TISH_FAST_NATIVE_BUILD", "1")
        .args(["build", fixture().to_str().unwrap(), "-o", out_bin.to_str().unwrap()])
        .args(["--feature", features])
        .output()
        .expect("spawn tish build");
    assert!(
        out.status.success(),
        "tish build --feature {features} failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let run = Command::new(&out_bin).output().expect("run built binary");
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
    String::from_utf8_lossy(&run.stdout).into_owned()
}

fn vm_run(features: &str) -> String {
    let tish = PathBuf::from(env!("CARGO_BIN_EXE_tish"));
    let out = Command::new(&tish)
        .args(["run", "--feature", features, fixture().to_str().unwrap()])
        .output()
        .expect("spawn tish run");
    assert!(
        out.status.success(),
        "tish run --feature {features} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn native_dune_cloud_run_features_without_await() {
    assert_eq!(build_and_run("http,fs,process"), EXPECTED);
}

#[test]
fn native_fs_only_without_await() {
    assert_eq!(build_and_run("fs"), EXPECTED);
}

#[test]
fn native_ws_only_without_await() {
    assert_eq!(build_and_run("ws"), EXPECTED);
}

#[test]
fn vm_fs_and_ws_have_promise() {
    assert_eq!(vm_run("fs"), EXPECTED);
    assert_eq!(vm_run("ws"), EXPECTED);
    assert_eq!(vm_run("http"), EXPECTED);
}
