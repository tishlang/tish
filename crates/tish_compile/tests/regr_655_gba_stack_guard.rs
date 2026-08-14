//! #655 — the stack-overflow guard must fire on GBA, but only when SP is on the
//! user stack, and a trip must be loud.
//!
//! Issue acceptance (ordered):
//! 1. A real no_std floor from the link map so deep call chains bail instead of
//!    overwriting agb IWRAM.
//! 2. Fail loudly — a silent `Value::Null` / NaN unwind on cartridge is
//!    indistinguishable from a codegen bug.
//!
//! Extra premise (learned the hard way): a floor from `__iwram_end` is only
//! meaningful when SP is inside IWRAM above that symbol. Without that band check
//! `sp < floor` is true on every call and the ROM dies blank before drawing.

use std::fs;
use std::path::PathBuf;

use tishlang_compile::{compile_project_full_emit, NativeEmitMode};

fn gba_rust(fixture: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(fixture);
    compile_project_full_emit(&path, path.parent(), &[], true, NativeEmitMode::Gba, None)
        .expect("compile for gba")
        .0
}

fn runtime_gba_lib() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../tish_runtime_gba/src/lib.rs");
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// A self-recursive typed fn opens with the stack check.
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
        copy0.contains("tishlang_runtime::stack_low()"),
        "gba typed recursion must open with the stack check (#655):\n{copy0}"
    );
}

/// Runtime contract for #655: band-gated floor + loud abort (not silent Null).
#[test]
fn gba_runtime_stack_guard_is_band_gated_and_loud() {
    let src = runtime_gba_lib();
    assert!(
        src.contains("sp_below_stack_floor")
            || (src.contains("IWRAM_EXCLUSIVE_END") && src.contains("STACK_HEADROOM")),
        "runtime must expose a band-gated floor predicate (#655)"
    );
    assert!(
        src.contains("0x0300_8000") || src.contains("0x03008000"),
        "runtime must bound the check to the IWRAM top (#655)"
    );
    assert!(
        src.contains("abort_on_stack_exhaustion") || src.contains("panic!"),
        "a tripped guard on GBA must abort loudly, not park a silent Null (#655)"
    );
    assert!(
        src.contains("enter_call_guarded") && src.contains("stack_low"),
        "boxed frames must still go through enter_call_guarded → stack_low (#655)"
    );
    // The silent host-shaped bail must not be the GBA trip path.
    let enter = src
        .split("pub fn enter_call_guarded")
        .nth(1)
        .expect("enter_call_guarded")
        .split("pub fn ")
        .next()
        .unwrap();
    assert!(
        !enter.contains("set_pending_throw(stack_overflow_error())"),
        "GBA enter_call_guarded must not silently park RangeError+Null on trip (#655):\n{enter}"
    );
}
