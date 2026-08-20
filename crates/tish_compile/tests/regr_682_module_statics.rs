//! #682 — module-scope state must have a top-level home, not a `run()` stack slot.
//!
//! Every module of a program is flattened into one `run()`, so every top-level function's cell,
//! its boxed closure, and one `_ref` clone per capturing sibling are live locals of a single frame.
//! On a real GBA game that frame reached 27–30 KB against a 32.5 KB IWRAM stack, and it grows with
//! program size — the ROM stops booting and no further feature can be added.

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

const MOD: &str = "\
export let hits: i32 = 0
export let bag = { hits: 0, tag: \"b\" }
export function tally(a: i32): i32 { hits = hits + a; bag = { hits: bag.hits + 1, tag: bag.tag }; return hits }
export function twice(a: i32): i32 { return tally(a) + tally(a) }
";

const ENTRY: &str = "\
import { tally, twice, hits, bag } from './pkg/m.tish'
function main() { console.log(twice(2) + tally(1) + hits + bag.hits) }
";

fn compile_in(dir: &str, mode: NativeEmitMode) -> String {
    let entry = fixture(dir, &[("main.tish", ENTRY), ("pkg/m.tish", MOD)]);
    compile_project_full_emit(&entry, entry.parent(), &[], true, mode, None)
        .expect("compile")
        .0
}

fn run_body(rust: &str) -> String {
    rust.split("fn run()").nth(1).expect("run()").to_string()
}

/// GBA has no `thread_local!` (no_std), so module fns land in `SingleCore` statics — the same split
/// #594 made for the numeric globals, and for the same reason: `with()` mirrors `LocalKey::with`,
/// so the read sites are written once and work on both targets.
#[test]
fn gba_module_fns_live_in_singlecore_statics() {
    let rust = compile_in("regr682_gba", NativeEmitMode::Gba);
    assert!(
        rust.contains(
            "static GF_TWICE: tishlang_runtime::SingleCore<core::cell::RefCell<Value>> = \
             tishlang_runtime::SingleCore::new(core::cell::RefCell::new(Value::Null));"
        ),
        "expected a SingleCore static for `twice`:\n{rust}"
    );
    let body = run_body(&rust);
    assert!(
        body.contains("GF_TWICE.with(|c| *c.borrow_mut() = {"),
        "the closure must be built INTO the static, not into a `run()` local:\n{body}"
    );
    assert!(
        !body.contains("let twice_cell"),
        "a promoted fn must not also keep a `run()` cell — that is the slot #682 is about:\n{body}"
    );
}

/// The host storage is `thread_local!`, one macro invocation per static: the macro recurses once
/// per item and a real program has hundreds of module fns, which blows rustc's default
/// `recursion_limit = 128` if they share one block.
#[test]
fn host_module_fns_live_in_thread_local_statics() {
    let rust = compile_in("regr682_host", NativeEmitMode::DesktopBin);
    assert!(
        rust.contains(
            "thread_local! { static GF_TALLY: std::cell::RefCell<Value> = \
             const { std::cell::RefCell::new(Value::Null) }; }"
        ),
        "expected a one-per-static thread_local for `tally`:\n{rust}"
    );
    assert!(
        !run_body(&rust).contains("let tally_cell"),
        "promoted fns keep no `run()` cell:\n{}",
        run_body(&rust)
    );
}

/// A sibling call inside a promoted closure reads the static instead of cloning a `_ref` in — that
/// clone is a per-closure slot in the SAME frame, so removing it is half the point.
#[test]
fn a_sibling_call_reads_the_static_instead_of_capturing_a_ref() {
    for mode in [NativeEmitMode::Gba, NativeEmitMode::DesktopBin] {
        let body = run_body(&compile_in(
            match mode {
                NativeEmitMode::Gba => "regr682_sib_gba",
                _ => "regr682_sib_host",
            },
            mode,
        ));
        assert!(
            body.contains("GF_TALLY.with(|c| (*c.borrow()).clone())"),
            "`twice` must reach `tally` through the static ({mode:?}):\n{body}"
        );
        assert!(
            !body.contains("let tally_ref"),
            "no `_ref` capture clone should survive ({mode:?}):\n{body}"
        );
    }
}

/// A module DATA binding that closures capture moves its CELL into a static. The static holds the
/// `VmRef`, not the value, so every read/write site is unchanged — what goes away is the `_cell`
/// clone each enclosing scope had to leave in its frame to thread the cell down.
#[test]
fn a_captured_module_binding_takes_its_cell_from_a_static() {
    for (dir, mode) in [
        ("regr682_var_gba", NativeEmitMode::Gba),
        ("regr682_var_host", NativeEmitMode::DesktopBin),
    ] {
        let rust = compile_in(dir, mode);
        let handle = "GV_BAG.with(|c| c.get_or_init(|| VmRef::new(Value::Null)).clone())";
        assert!(
            rust.contains(handle),
            "`bag`'s cell must come from its static ({mode:?}):\n{rust}"
        );
        let body = run_body(&rust);
        assert!(
            !body.contains("let bag_cell"),
            "no `_cell` clone should be threaded through the frame ({mode:?}):\n{body}"
        );
        assert!(
            body.contains("*bag.borrow_mut()"),
            "the write path is unchanged — it still sees a `VmRef` local ({mode:?}):\n{body}"
        );
    }
}

/// A NATIVELY typed module binding keeps its representation. Boxing a `: i32` scalar into the one
/// uniform `VmRef<Value>` static would trade a stack slot for a per-read `Value` clone on the GBA
/// hot path — that binding's home belongs to the typed-lowering work (#631 / #647 / #654).
#[test]
fn a_natively_typed_module_binding_is_left_alone() {
    let rust = compile_in("regr682_typed", NativeEmitMode::Gba);
    assert!(
        !rust.contains("GV_HITS"),
        "`hits: i32` must not be promoted into the boxed static:\n{rust}"
    );
    assert!(
        run_body(&rust).contains("let hits_cell"),
        "…and keeps today's capture-cell emission:\n{}",
        run_body(&rust)
    );
}
