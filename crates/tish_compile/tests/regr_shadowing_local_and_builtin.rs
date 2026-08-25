//! Two name-resolution shadowing bugs found via the chuggie-engine examples.
//!
//! 1. A LOCAL shadows a same-named MODULE FUNCTION from another bundled module. earshot declared
//!    `let dist: i32` while packages/sfx.tish had a non-exported `function dist(...)`; in
//!    call-argument position the argument lowered to `__TISH_GF_dist.with(...)` — the function
//!    cell — instead of the local, and the callee panicked "expected number" at runtime.
//!
//! 2. A USER FUNCTION shadows a BUILTIN of the same name. pong-link's `function serve(toRight)`
//!    (serve is the HTTP builtin) compiled to sibling capture preludes `let serve = serve.clone();`
//!    with no `serve` binding in scope — E0425 at rustc time.

use std::path::PathBuf;

use tishlang_compile::{compile_project_full_emit, NativeEmitMode};

fn fixture(dir: &str, files: &[(&str, &str)]) -> PathBuf {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target")
        .join(dir);
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("pkg")).unwrap();
    for (rel, body) in files {
        std::fs::write(root.join(rel), body).unwrap();
    }
    root.join("main.tish")
}

fn compile(dir: &str, files: &[(&str, &str)], mode: NativeEmitMode) -> String {
    let entry = fixture(dir, files);
    compile_project_full_emit(&entry, entry.parent(), &[], true, mode, None)
        .expect("compile")
        .0
}

// ── 1. local shadows a module-internal function of another module ────────────────────────────────

const SFX_MOD: &str = "\
function dist(dx: i32, dy: i32): i32 { return dx + dy }
export function volumeAt(d: i32): i32 { return 256 - dist(d, 0) }
";

const SHADOW_ENTRY: &str = "\
import { volumeAt } from './pkg/sfx.tish'
function paint() {
  let dist: i32 = 96
  console.log(\"vol \" + volumeAt(dist))
}
paint()
";

#[test]
fn local_shadows_module_fn_in_argument_position() {
    for (mode, dir) in [
        (NativeEmitMode::Gba, "regr_shadow_dist_gba"),
        (NativeEmitMode::DesktopBin, "regr_shadow_dist_host"),
    ] {
        let rust = compile(
            dir,
            &[("main.tish", SHADOW_ENTRY), ("pkg/sfx.tish", SFX_MOD)],
            mode,
        );
        // paint()'s call must pass the LOCAL, never the module function's cell. The cell read
        // is fine inside volumeAt itself (calling its sibling), so scope the assertion to the
        // argument list of the volumeAt call.
        let call = rust
            .split("__TISH_GF_volumeAt.with(|c| (*c.borrow()).clone())")
            .nth(1)
            .or_else(|| rust.split("volumeAt").nth(1))
            .expect("volumeAt call site");
        let args = &call[..call.len().min(400)];
        assert!(
            !args.contains("__TISH_GF_dist"),
            "{mode:?}: volumeAt's argument resolved to the module fn cell instead of the local:\n{args}"
        );
    }
}

// ── 2. user function shadows a builtin name ──────────────────────────────────────────────────────

const SERVE_ENTRY: &str = "\
function serve(toRight: i32): i32 { return toRight + 1 }
function resetMatch(): i32 { return serve(1) }
console.log(\"r \" + resetMatch())
";

#[test]
fn user_fn_shadows_builtin_name() {
    for (mode, dir) in [
        (NativeEmitMode::Gba, "regr_shadow_serve_gba"),
        (NativeEmitMode::DesktopBin, "regr_shadow_serve_host"),
    ] {
        let rust = compile(dir, &[("main.tish", SERVE_ENTRY)], mode);
        // The bug emitted `let serve = serve.clone();` with no such binding in scope. If a
        // capture prelude names `serve`, a definition for it must exist (a GF cell or a local
        // `let serve =` initialisation), or rustc dies with E0425.
        if rust.contains("let serve = serve.clone();") {
            assert!(
                rust.contains("__TISH_GF_serve")
                    || rust.contains("let serve = Value::native")
                    || rust.contains("let serve = {"),
                "{mode:?}: capture prelude references `serve` but nothing defines it (E0425):\n\
                 the user declaration must shadow the builtin"
            );
        }
    }
}
