//! A native build names its executable after `-o`; it must not also leave one in the Tish checkout's own
//! `target/release/`, where `-o tish` replaced the compiler and `-o sh` shadowed the system shell for anything
//! with that directory on `PATH`. Builds go through `target/tish-native/` instead (`native_target_dir`).

use std::path::PathBuf;
use std::process::Command;

fn workspace() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

#[test]
fn native_outputs_never_land_in_target_release() {
    let release = workspace().join("target/release");
    let compiler = release.join("tish");
    let before = std::fs::read(&compiler).ok();
    let had_sh = release.join("sh").exists();

    let dir = std::env::temp_dir().join(format!("tish-out-names-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("hello.tish");
    std::fs::write(&src, "console.log(\"hi\")\n").unwrap();
    for name in ["tish", "sh"] {
        let out = Command::new(env!("CARGO_BIN_EXE_tish"))
            .args(["build", src.to_str().unwrap(), "--target", "native", "-o", dir.join(name).to_str().unwrap()])
            .env("TISHLANG_WORKSPACE", workspace())
            .env_remove("TISH_NATIVE_TARGET_DIR")
            .output()
            .expect("spawn tish build");
        assert!(out.status.success(), "build -o {name}: {}", String::from_utf8_lossy(&out.stderr));
        let run = Command::new(dir.join(name)).output().expect("run the built program");
        assert_eq!(String::from_utf8_lossy(&run.stdout), "hi\n", "-o {name} builds the program");
    }
    let _ = std::fs::remove_dir_all(&dir);

    assert_eq!(std::fs::read(&compiler).ok(), before, "-o tish must not touch target/release/tish");
    assert_eq!(release.join("sh").exists(), had_sh, "-o sh must not create target/release/sh");
}
