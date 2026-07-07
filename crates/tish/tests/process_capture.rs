//! Issue #475: `process.execCapture` / `execFileCapture` return `{ code, stdout, stderr }` identically
//! on every backend (interpreter, bytecode VM, native AOT).

use std::path::PathBuf;
use std::process::Command;

fn run_on(backend: &str) -> String {
    let tish = PathBuf::from(env!("CARGO_BIN_EXE_tish"));
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("process_capture.tish");
    let out = Command::new(&tish)
        .args([
            "run",
            "--backend",
            backend,
            "--feature",
            "process",
            path.to_str().unwrap(),
        ])
        .output()
        .expect("spawn tish run");
    assert!(
        out.status.success(),
        "tish run --backend {backend} process_capture.tish failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

const EXPECTED: &str = "ok=0,hi,true\nbad=1\nmiss=-1,true\nshell=0,a,b\ndone\n";

#[test]
#[cfg(unix)]
fn exec_capture_on_every_backend() {
    for backend in ["vm", "interp", "native"] {
        assert_eq!(run_on(backend), EXPECTED, "exec-capture mismatch on {backend}");
    }
}
