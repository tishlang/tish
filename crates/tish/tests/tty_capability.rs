//! Issue #101: the `tish:tty` capability (raw mode, terminal size, key/resize events) is
//! available on every backend behind the `tty` feature. CI has no real terminal, so this
//! asserts the module loads and degrades gracefully (size → null, isTTY → a boolean, the
//! event/raw-mode primitives are functions). Interactive behavior is covered manually.

use std::path::PathBuf;
use std::process::Command;

const EXPECTED: &str = "\
isTTY: boolean
size: null
setRawMode: function
read: function
readLine: function
alt: function function
";

fn run(backend: &str, fixture: &str) -> String {
    let tish = PathBuf::from(env!("CARGO_BIN_EXE_tish"));
    let out = Command::new(&tish)
        .args(["run", "--backend", backend, "--feature", "tty", fixture])
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
fn tty_module_loads_and_degrades_without_a_terminal() {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("tty_capability.tish");
    assert!(fixture.is_file(), "missing fixture {}", fixture.display());
    let fixture = fixture.to_str().unwrap();

    assert_eq!(run("vm", fixture), EXPECTED, "vm output mismatch");
    assert_eq!(run("interp", fixture), EXPECTED, "interp output mismatch");
}
