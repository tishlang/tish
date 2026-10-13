//! `tish:timers` and `tish:encoding` give the same results on every target, so one Tish source can
//! run natively and as JavaScript: the bytecode VM, a native build, and `--target js` (ES modules and a
//! bundle) run through Node when it is installed.
//!
//! - `delay(ms)`: a promise that settles with null once `ms` has passed since it was made, for bounded
//!   waits (`Promise.race([work, delay(ms)])`), with `Promise.spawn` as the work.
//! - `utf8Complete(bytes)`: how many leading bytes end on a whole character, for decoding a stream.

use std::path::{Path, PathBuf};
use std::process::Command;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)
}

fn tish(args: &[&str]) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_tish")).args(args).output().expect("spawn tish");
    assert!(out.status.success(), "tish {args:?} failed: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn node(script: &Path) -> String {
    let out = Command::new("node").arg(script).output().expect("run node");
    assert!(out.status.success(), "node {} failed: {}", script.display(), String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn node_available() -> bool {
    Command::new("node").arg("--version").output().map(|o| o.status.success()).unwrap_or(false)
}

/// Run `name` on every target and check each prints `expected`.
fn same_everywhere(name: &str, features: &str, expected: &str) {
    let src = fixture(name);
    let src = src.to_str().unwrap();
    for backend in ["vm", "native"] {
        assert_eq!(tish(&["run", "--backend", backend, "--feature", features, src]), expected, "{name} on {backend}");
    }
    if !node_available() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("tish-portable-{}-{}", std::process::id(), name));
    let _ = std::fs::remove_dir_all(&dir);
    let esm = dir.join("esm");
    tish(&["build", src, "--target", "js", "--format", "esm", "-o", esm.to_str().unwrap()]);
    std::fs::write(esm.join("package.json"), r#"{"type":"module"}"#).unwrap();
    let entry = esm.join(Path::new(name).with_extension("js"));
    assert_eq!(node(&entry), expected, "{name} as ES modules");
    let bundle = dir.join("bundle.mjs");
    tish(&["build", src, "--target", "js", "-o", bundle.to_str().unwrap()]);
    assert_eq!(node(&bundle), expected, "{name} as a bundle");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn delay_bounds_waits_everywhere() {
    same_everywhere(
        "timers_delay.tish",
        "timers,promise",
        "in time: answer\ntimed out: null\nfinished: b,c,d,a\ndeadline fixed: true\n",
    );
}

#[test]
fn encoding_is_the_same_everywhere() {
    same_everywhere(
        "encoding_portable.tish",
        "encoding",
        "héllo ✓\na?b>c\naGVsbG8gd29ybGQ= aGk_\nnull\ncafé ✓ 🙂 done\n",
    );
}
