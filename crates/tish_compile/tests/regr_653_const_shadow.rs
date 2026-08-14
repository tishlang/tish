//! #653 — same-named consts from two modules must not silently shadow in the flat native scope.
//!
//! Issue: two modules export `WALK` (mode vs sheet slot). Flattened to one Rust block, the later
//! `let mut WALK` shadows the earlier — host interp is correct, native/GBA is wrong, no error.
//!
//! Acceptance (any of): module-scoped resolution, uniquely-mangled items, or a hard error.
//! This delivery uniquely-mangles every non-entry top-level const/let so an importer binds to
//! ITS module's symbol.

use std::fs;

use tishlang_compile::{compile_project_full_emit, NativeEmitMode};

#[test]
fn two_modules_exporting_walk_keep_distinct_bindings() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::write(
        root.join("hero.tish"),
        "export const WALK = 1\nexport function modeOf() { return WALK }\n",
    )
    .unwrap();
    fs::write(
        root.join("sheet.tish"),
        "export const WALK = 0\nexport function slotOf() { return WALK }\n",
    )
    .unwrap();
    fs::write(
        root.join("main.tish"),
        "import { modeOf } from \"./hero.tish\"\n\
         import { slotOf } from \"./sheet.tish\"\n\
         console.log(modeOf())\n\
         console.log(slotOf())\n",
    )
    .unwrap();
    let path = root.join("main.tish");
    let rust = compile_project_full_emit(&path, path.parent(), &[], true, NativeEmitMode::Gba, None)
        .expect("compile")
        .0;

    let walk_lines: Vec<&str> = rust.lines().filter(|l| l.contains("WALK")).collect();
    let dump = walk_lines.join("\n");

    // A single bare `WALK` binding is the silent-shadow failure mode.
    let bare_walk_lets = rust
        .lines()
        .filter(|l| {
            let t = l.trim_start();
            (t.starts_with("let mut WALK") || t.starts_with("let WALK") || t.starts_with("static G_WALK:"))
                && !l.contains("WALK__")
        })
        .count();
    assert!(
        bare_walk_lets <= 1,
        "two bare WALK bindings in one scope silently shadow (#653):\n{dump}"
    );

    // Distinct module values (hero=1, sheet=0) must both survive under distinct symbols.
    assert!(
        rust.contains("G_WALK__M0") && rust.contains("G_WALK__M1"),
        "both modules' WALK bindings must survive as distinct mangled symbols (#653):\n{dump}"
    );
    assert!(
        dump.contains("1_f64") && dump.contains("0_f64"),
        "both original values must remain (#653):\n{dump}"
    );
    // Each helper reads its own module's global — not a shared shadowed name.
    assert!(
        dump.contains("G_WALK__M0.with") && dump.contains("G_WALK__M1.with"),
        "call sites must read through the mangled per-module bindings (#653):\n{dump}"
    );
}
