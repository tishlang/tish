//! #663 — passing a typed module array to a call must not de-optimise EVERY read of it.
//!
//! `collect_escaping_array_names` (#597) boxed an annotated array as soon as it was passed as a call
//! argument, to preserve aliasing. But being forwarded is only an aliasing hazard if the CALLEE can
//! write through the reference or let it escape; when it cannot, boxing buys nothing and costs a
//! boxed read on every OTHER access — usually a hot loop, while the call that caused it may run once
//! per level. Measured at 3.8x per read in the report.
//!
//! These assert the SPLIT: a read-only callee leaves the array typed, and every aliasing shape still
//! boxes. The aliasing half is the one that must not regress — getting it wrong is a silent wrong
//! answer, not a slow one.

use std::path::PathBuf;

use tishlang_compile::{compile_project_full_emit, NativeEmitMode};

fn gba_rust(fixture: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join(fixture);
    compile_project_full_emit(&path, path.parent(), &[], true, NativeEmitMode::Gba, None)
        .expect("compile for gba")
        .0
}

/// `A.borrow()` + `.get(i).copied()` is the typed read; `get_index(&vm_read(&A), ..)` is the boxed one.
fn is_typed(rust: &str, name: &str) -> bool {
    rust.contains(&format!("let __bg = {name}.borrow()"))
}

#[test]
fn readonly_callee_leaves_the_array_typed() {
    let rust = gba_rust("fixtures_663_array_escape.tish");
    assert!(
        is_typed(&rust, "F"),
        "F is passed only to a read-only callee (`arr.length`), so it must keep the typed read \
         path — boxing it costs 3.8x on every read for a call that cannot alias it (#663)"
    );
}

#[test]
fn aliasing_callees_still_box() {
    let rust = gba_rust("fixtures_663_array_escape.tish");
    for (name, why) in [
        ("C", "the callee WRITES an element (`arr[0] = 99`)"),
        ("D", "the callee FORWARDS it to a mutating fn"),
        ("E", "the callee lets it ESCAPE (stores it in a module var)"),
    ] {
        assert!(
            !is_typed(&rust, name),
            "{name} must stay boxed — {why}, so a typed (copied) representation would silently \
             lose a caller-visible mutation (#663/#597)"
        );
    }
}
