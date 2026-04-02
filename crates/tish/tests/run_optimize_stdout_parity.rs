//! Ensure `tish run` and `tish run --no-optimize` agree on stdout for the same program.

use std::path::PathBuf;
use std::process::Command;

#[test]
fn string_or_fixture_stdout_matches_with_and_without_optimize() {
    let tish = PathBuf::from(env!("CARGO_BIN_EXE_tish"));
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("tish_vm")
        .join("tests")
        .join("fixtures")
        .join("or_string_cmd.tish");
    assert!(fixture.is_file(), "missing fixture {}", fixture.display());

    let out_default = Command::new(&tish)
        .args([
            "run",
            "--feature",
            "process",
            fixture.to_str().unwrap(),
        ])
        .output()
        .expect("spawn tish run");
    assert!(
        out_default.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out_default.stderr)
    );

    let out_noopt = Command::new(&tish)
        .args([
            "run",
            "--no-optimize",
            "--feature",
            "process",
            fixture.to_str().unwrap(),
        ])
        .output()
        .expect("spawn tish run --no-optimize");
    assert!(
        out_noopt.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out_noopt.stderr)
    );

    assert_eq!(
        out_default.stdout, out_noopt.stdout,
        "stdout differs:\n default: {:?}\n noopt: {:?}",
        String::from_utf8_lossy(&out_default.stdout),
        String::from_utf8_lossy(&out_noopt.stdout)
    );
}
