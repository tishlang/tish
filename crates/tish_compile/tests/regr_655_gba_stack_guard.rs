//! #655 — the stack-overflow guard must actually be emitted on the GBA target.
//!
//! The recursion guard was gated OFF for `NativeEmitMode::Gba` (the no_std runtime had no
//! stack-pressure probe), and the boxed guard's floor fell back to "never trips". GBA is the one
//! target where that is unrecoverable: 32 KB of IWRAM, no MMU, no guard page — an unguarded
//! overflow grows down into agb's live data (the audio mixer's buffers among them) and surfaces
//! frames later as an illegal opcode at an address that names nothing.
//!
//! `tishlang_runtime_gba` now derives a real floor from the link map (`__iwram_end`), so both the
//! typed and boxed guards are emitted here exactly as they are on the host.

use std::path::PathBuf;

use tishlang_compile::{compile_project_full_emit, NativeEmitMode};

fn gba_rust(fixture: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(fixture);
    compile_project_full_emit(&path, path.parent(), &[], true, NativeEmitMode::Gba, None)
        .expect("compile for gba")
        .0
}

/// A self-recursive typed fn opens with the stack check and bails through the NaN sentinel.
#[test]
fn gba_typed_recursion_emits_stack_guard() {
    let rust = gba_rust("../../tests/perf/recursion_fib.tish");
    let copy0 = rust
        .split("fn fib_native(")
        .nth(1)
        .expect("fib_native must be lowered on gba")
        .split("\nfn ")
        .next()
        .unwrap()
        .to_string();
    assert!(
        copy0.contains("tishlang_runtime::stack_low()")
            && copy0.contains("tishlang_runtime::recursion_tripped_f64()"),
        "gba typed recursion must open with the stack check — without it an overflow silently \
         overwrites IWRAM instead of raising a catchable RangeError:\n{copy0}"
    );
}
