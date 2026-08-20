//! #682 — `TISH_MODULE_STATICS=0` restores the pre-change emission, so a regression can be
//! bisected without a revert.
//!
//! Its own test binary because it sets a process-wide env var: sharing a binary with the other
//! #682 tests would let it flip their answers mid-run.

use std::path::PathBuf;

use tishlang_compile::{compile_project_full_emit, NativeEmitMode};

#[test]
fn the_kill_switch_restores_run_locals() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/regr682_off");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("pkg")).unwrap();
    std::fs::write(
        root.join("pkg/m.tish"),
        "export let hits: i32 = 0\n\
         export function tally(a: i32): i32 { hits = hits + a; return hits }\n\
         export function twice(a: i32): i32 { return tally(a) + tally(a) }\n",
    )
    .unwrap();
    std::fs::write(
        root.join("main.tish"),
        "import { tally, twice, hits } from './pkg/m.tish'\n\
         function main() { console.log(twice(2) + tally(1) + hits) }\n",
    )
    .unwrap();

    std::env::set_var("TISH_MODULE_STATICS", "0");
    let entry = root.join("main.tish");
    let rust = compile_project_full_emit(
        &entry,
        entry.parent(),
        &[],
        true,
        NativeEmitMode::Gba,
        None,
    )
    .expect("compile")
    .0;
    std::env::remove_var("TISH_MODULE_STATICS");

    assert!(
        !rust.contains("static GF_"),
        "the kill switch must emit no module-fn statics:\n{rust}"
    );
    let body = rust.split("fn run()").nth(1).expect("run()");
    assert!(
        body.contains("let twice_cell"),
        "…and must restore the `run()` cell:\n{body}"
    );
}
