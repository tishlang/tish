//! Issue #74: a runtime error from the bytecode VM carries its source location (`file:line`)
//! instead of a bare message, so embedders/users can find where it came from.

use std::path::PathBuf;
use std::process::Command;

#[test]
fn runtime_error_reports_source_file_and_line() {
    let tish = PathBuf::from(env!("CARGO_BIN_EXE_tish"));
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("runtime_error_location.tish");
    assert!(fixture.is_file(), "missing fixture {}", fixture.display());

    // `obj.field.deep` reads `.field` of `null` on line 4 of the fixture.
    let out = Command::new(&tish)
        .args(["run", "--backend", "vm", fixture.to_str().unwrap()])
        .output()
        .expect("spawn tish run");
    assert!(!out.status.success(), "expected a runtime error");
    let stderr = String::from_utf8_lossy(&out.stderr);

    assert!(
        stderr.contains(":4"),
        "error should report the source line (4):\n{stderr}"
    );
    assert!(
        stderr.contains("runtime_error_location.tish"),
        "error should report the source file:\n{stderr}"
    );
    assert!(
        stderr.contains("Cannot read property 'field' of null"),
        "error should keep the original message:\n{stderr}"
    );
}
