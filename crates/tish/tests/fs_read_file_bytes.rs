//! Issue #120: `tish:fs.readFileBytes` reads a binary file as an array of byte values (0–255)
//! on every backend, where the UTF-8-only `readFile` cannot. Verifies the exact bytes,
//! including a NUL and 0xFF (not valid UTF-8).

use std::path::PathBuf;
use std::process::Command;

const EXPECTED: &str = "\
isArray:true
len:7
bytes:0,1,2,255,128,72,73
";

fn run(backend: &str, fixture: &str, data_path: &str) -> String {
    let tish = PathBuf::from(env!("CARGO_BIN_EXE_tish"));
    let out = Command::new(&tish)
        .args(["run", "--backend", backend, "--feature", "fs,process", fixture])
        .env("RFB_FIXTURE", data_path)
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
fn read_file_bytes_reads_binary_on_every_backend() {
    // A file whose bytes are not valid UTF-8 (NUL, 0xFF, 0x80) — readFile would error on it.
    let data_path = std::env::temp_dir().join("tish_rfb_fixture.dat");
    std::fs::write(&data_path, [0u8, 1, 2, 255, 128, 72, 73]).expect("write fixture");
    let data_path = data_path.to_str().unwrap();

    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("read_file_bytes.tish");
    assert!(fixture.is_file(), "missing fixture {}", fixture.display());
    let fixture = fixture.to_str().unwrap();

    assert_eq!(run("vm", fixture, data_path), EXPECTED, "vm output mismatch");
    assert_eq!(run("interp", fixture, data_path), EXPECTED, "interp output mismatch");
}
