//! `tish:crypto` / `tish:encoding` behave the same on the bytecode VM (`tish run`, `tish test`)
//! as in native builds; they used to exist only in the native backend. Also `tish:fs` `chmod` on
//! the VM.

use std::path::PathBuf;
use std::process::Command;

fn tish() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_tish"))
}

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

fn run_on(backend: &str) -> String {
    let out = Command::new(tish())
        .args([
            "run",
            "--backend",
            backend,
            "--feature",
            "crypto,encoding",
            fixture("crypto_encoding.tish").to_str().unwrap(),
        ])
        .output()
        .expect("spawn tish run");
    assert!(
        out.status.success(),
        "tish run --backend {backend} crypto_encoding.tish failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

const EXPECTED: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad\n\
ungWv48Bz-pBQUDeXa4iI7ADYaOWF3qctBD_YfIAFa0\n\
random=32\n\
héllo ✓\n\
a?b>c\n";

#[test]
fn crypto_and_encoding_on_vm_and_native() {
    for backend in ["vm", "native"] {
        assert_eq!(run_on(backend), EXPECTED, "crypto/encoding mismatch on {backend}");
    }
}

#[test]
#[cfg(unix)]
fn vm_chmod_sets_file_mode() {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!("tish-chmod-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("secret.txt");
    let script = dir.join("chmod.tish");
    std::fs::write(
        &script,
        format!(
            "import {{ writeFile, chmod }} from \"tish:fs\"\nwriteFile({:?}, \"x\")\nchmod({:?}, 384)\n",
            file.to_str().unwrap(),
            file.to_str().unwrap()
        ),
    )
    .unwrap();
    let out = Command::new(tish())
        .args(["run", "--backend", "vm", "--feature", "fs", script.to_str().unwrap()])
        .output()
        .expect("spawn tish run");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let mode = std::fs::metadata(&file).unwrap().permissions().mode() & 0o777;
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(mode, 0o600);
}
