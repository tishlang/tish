//! #672 — a `declare fn` parameter marked `readonly` lets the caller's array stay typed.
//!
//! #663 stopped boxing an escaping array when the callee can be proven not to alias it, but its
//! predicate returns false for a `cargo:` native — the body is invisible, and conservative is the
//! only safe guess. So an array handed to a native once per level was still boxed on every read
//! everywhere in the program (measured 3.8x: 1.55 ticks/read vs 5.87).
//!
//! A native's contract is already declared, though, and `readonly` is the crate author stating the
//! part the compiler cannot see. Opt-in: an unmarked parameter keeps today's conservative boxing,
//! because a native that DOES write through its argument must stay correct and silence has to keep
//! meaning "assume it might".

use std::path::PathBuf;

use tishlang_compile::{compile_project_full_emit, NativeEmitMode};

fn gba_rust(fixture: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join(fixture);
    compile_project_full_emit(&path, path.parent(), &[], true, NativeEmitMode::Gba, None)
        .expect("compile for gba")
        .0
}

fn is_typed(rust: &str, name: &str) -> bool {
    rust.contains(&format!("let __bg = {name}.borrow()"))
}

#[test]
fn readonly_native_param_keeps_the_array_typed() {
    let rust = gba_rust("fixtures_672_readonly_native.tish");
    assert!(
        is_typed(&rust, "A"),
        "A is never forwarded and must stay typed (#663 baseline)"
    );
    assert!(
        is_typed(&rust, "B"),
        "B is forwarded to a native whose `declare fn` marks that parameter `readonly`, so it must \
         keep the typed read path — this is the whole point of #672"
    );
}

#[test]
fn unmarked_native_param_still_boxes() {
    let rust = gba_rust("fixtures_672_readonly_native.tish");
    assert!(
        !is_typed(&rust, "C"),
        "C is forwarded to a native with NO `readonly` marker — its body is invisible, so it must \
         keep #663's conservative boxing. Defaulting to read-only here would silently drop a \
         native's writes to the caller's array (#672)"
    );
}
