//! #654 — an immutable captured scalar must not be re-read out of a `VmRef` cell on every call.
//!
//! Issue: a module-level `const` that is never reassigned was still lowered to a `VmRef` cell and
//! re-borrowed + cloned on EVERY call. For a per-frame function that is pure tax, and it scales
//! with how many constants the function names.
//!
//! Delivery: only proven immutable SCALAR literals (and aliases of them) skip the cell. Aggregates
//! keep a cell — a by-value snapshot of a typed Vec/struct at closure creation goes stale when
//! another function mutates the module binding.

use std::fs;

use tishlang_compile::{compile_project_full_emit, NativeEmitMode};

#[test]
fn immutable_scalar_capture_needs_no_cell() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::write(
        root.join("hero.tish"),
        "export const HERO_NAME = \"link\"\nexport const HERO_WALK = 1\n",
    )
    .unwrap();
    fs::write(
        root.join("main.tish"),
        "import { HERO_NAME, HERO_WALK } from \"./hero.tish\"\n\
         fn heroAnim(t) { return HERO_NAME + t + HERO_WALK }\n\
         fn main() { console.log(heroAnim(1)) }\n\
         main()\n",
    )
    .unwrap();
    let path = root.join("main.tish");
    let rust = compile_project_full_emit(&path, path.parent(), &[], true, NativeEmitMode::Gba, None)
        .expect("compile")
        .0;

    assert!(
        !rust.contains("HERO_NAME_cell"),
        "an immutable captured string const must not get a VmRef cell (#654):\n{rust}"
    );
    assert!(
        rust.contains("HERO_NAME_capt") || rust.contains("let mut HERO_NAME = HERO_NAME_capt"),
        "the call body must bind the string const from a captured snapshot (#654)"
    );
}

#[test]
fn module_aggregate_capture_keeps_a_cell() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::write(
        root.join("main.tish"),
        "let boards = []\n\
         fn pushBoard(b) { boards.push(b) }\n\
         fn countBoards() { return boards.length }\n\
         pushBoard(1)\n\
         console.log(countBoards())\n",
    )
    .unwrap();
    let path = root.join("main.tish");
    let rust = compile_project_full_emit(&path, path.parent(), &[], true, NativeEmitMode::Gba, None)
        .expect("compile")
        .0;

    // If countBoards snapshotted `boards` by value, pushBoard's mutation would be invisible.
    assert!(
        !rust.contains("boards_capt"),
        "a module aggregate must not be by-value captured (#654 / stale-copy hazard):\n{}",
        rust.lines().filter(|l| l.contains("boards")).collect::<Vec<_>>().join("\n")
    );
    assert!(
        rust.contains("boards_cell") || rust.contains("VmRef::new"),
        "module aggregate captures must go through a shared cell (#654)"
    );
}
