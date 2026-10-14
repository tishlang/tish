//! `--feature timers` alone must make `delay` from `tish:timers` available in a native build.
//! `delay(ms)` returns a Promise, so `timers` links the runtime's promise machinery; before, the
//! generated crate called `tishlang_runtime::timer_delay`, which only exists with `promise`, and the
//! await helpers were never imported ("cannot find function `timer_delay`").
//!
//! Built with `tish build` and the binary run: `tish run --backend native` runs the bytecode VM.

use std::path::PathBuf;
use std::process::Command;

#[test]
fn delay_with_only_the_timers_feature() {
    let tish = PathBuf::from(env!("CARGO_BIN_EXE_tish"));
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/timers_delay.tish");
    let dir = tempfile::tempdir().expect("tempdir");
    let bin = dir.path().join("timers_delay");
    let out = Command::new(&tish)
        .env("TISH_FAST_NATIVE_BUILD", "1")
        .args(["build", src.to_str().unwrap(), "-o", bin.to_str().unwrap()])
        .args(["--feature", "timers"])
        .output()
        .expect("spawn tish build");
    assert!(
        out.status.success(),
        "tish build --feature timers failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let run = Command::new(&bin).output().expect("run built binary");
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
    assert_eq!(
        String::from_utf8_lossy(&run.stdout),
        "in time: answer\ntimed out: null\nfinished: b,c,d,a\ndeadline fixed: true\n"
    );
}
