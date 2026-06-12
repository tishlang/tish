//! Issue #122: Node `fs` parity on the bytecode VM (`tish run` default backend) — the sync
//! surface via `node:fs` and the async `node:fs/promises` surface (await + rejection).

use std::path::PathBuf;
use std::process::Command;

fn run(fixture: &str, tmp: &str) -> String {
    let tish = PathBuf::from(env!("CARGO_BIN_EXE_tish"));
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(fixture);
    let out = Command::new(&tish)
        .args(["run", "--feature", "fs,process", path.to_str().unwrap()])
        .env("FS_TMP", tmp)
        .output()
        .expect("spawn tish run");
    assert!(
        out.status.success(),
        "tish run {fixture} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn tmp_dir(tag: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let d = std::env::temp_dir().join(format!("tish_fs_parity_{tag}_{nanos:x}"));
    std::fs::create_dir_all(&d).unwrap();
    d.to_str().unwrap().to_string()
}

#[test]
fn node_fs_sync_surface() {
    let tmp = tmp_dir("sync");
    let out = run("fs_parity_sync.tish", &tmp);
    assert_eq!(
        out,
        "read=hi!\n\
         exists=true,false\n\
         size=3,file=true,dir=false\n\
         dir=x.txt\n\
         renamed=false,true\n\
         rm=false\n\
         const=4\n\
         done\n",
        "sync surface mismatch"
    );
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn node_fs_promises_surface() {
    let tmp = tmp_dir("prom");
    let out = run("fs_parity_promises.tish", &tmp);
    assert_eq!(
        out,
        "read=async hello\n\
         size=11\n\
         dir=x\n\
         access-missing-rejected=true\n\
         done\n",
        "promises surface mismatch"
    );
    let _ = std::fs::remove_dir_all(&tmp);
}
