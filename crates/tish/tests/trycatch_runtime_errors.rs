//! Issue #60: `try/catch` must catch VM-internal runtime errors (property/method access on
//! null, calling a non-function) AND `throw`s that propagate across function-call frames,
//! preserving the thrown value. Verified on the bytecode VM and the tree-walk interpreter
//! (the two backends #60 targets — the embedded/browser target is the VM, the interpreter is
//! its oracle). The native backend surfaces these as Rust panics and is tracked separately.

use std::path::PathBuf;
use std::process::Command;

const EXPECTED: &str = "\
A caught
B caught: boom
C caught
D done
E caught: rethrown
F ok
G total: 206
";

fn run(backend: &str, fixture: &str) -> String {
    let tish = PathBuf::from(env!("CARGO_BIN_EXE_tish"));
    let out = Command::new(&tish)
        .args(["run", "--backend", backend, fixture])
        .output()
        .expect("spawn tish run");
    assert!(
        out.status.success(),
        "tish run --backend {backend} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn trycatch_catches_runtime_and_cross_frame_errors() {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("trycatch_runtime_errors.tish");
    assert!(fixture.is_file(), "missing fixture {}", fixture.display());
    let fixture = fixture.to_str().unwrap();

    assert_eq!(run("vm", fixture), EXPECTED, "vm output mismatch");
    assert_eq!(run("interp", fixture), EXPECTED, "interp output mismatch");
}
