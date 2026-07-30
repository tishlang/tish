//! GBA `fixed` native coercions (the "gba perf" numeric lowering). These paths lift a native
//! integer/f64 into `tishlang_runtime::Fixed` (agb `Num<i32,8>`, raw = value << 8) WITHOUT a boxed
//! `Value` round-trip, so `fixed` arithmetic stays native:
//!   * `let v: fixed = <int/float literal>`          — literal → Fixed
//!   * `let d: fixed = t * speed` (t is a runtime i32) — runtime integer LIFTED to Fixed before `Fixed * Fixed`
//!   * `nx = <int literal>` (reassign a fixed local)  — RHS lifted to Fixed (the E0308 reassignment fix)
//!
//! All three are GBA-only (`fixed`/`i32` are native scalars only under `--target gba`). Off-GBA the
//! `fixed` annotation falls back to a boxed `Value`, so none of the native `Fixed` code appears —
//! asserted below so a leak into the desktop build (where `tishlang_runtime::Fixed` does not exist)
//! would fail the test rather than the downstream Rust compile.

use std::path::PathBuf;

use tishlang_compile::{compile_project_full, compile_project_full_emit, NativeEmitMode};

const SRC: &str = "\
let speed: fixed = 2
let t: i32 = 5
let d: fixed = t * speed
let nx: fixed = 0
nx = 3
console.log(d)
console.log(nx)
";

fn write_src() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    let path = dir.join("fixed_native_coercions.tish");
    std::fs::write(&path, SRC).unwrap();
    path
}

#[test]
fn gba_lifts_int_and_float_into_native_fixed() {
    let path = write_src();
    let rust = compile_project_full_emit(&path, path.parent(), &[], true, NativeEmitMode::Gba, None)
        .unwrap()
        .0;

    // `let speed: fixed = 2` — the int literal lifts to Fixed (× 256 via from_raw), not a boxed number.
    assert!(
        rust.contains("let mut speed: tishlang_runtime::Fixed = tishlang_runtime::Fixed::from_raw("),
        "`let speed: fixed = 2` must lower to a native Fixed::from_raw literal.\n{rust}"
    );
    // `t * speed` (runtime i32 × fixed) — the integer operand `t` is LIFTED to Fixed (raw = value * 256)
    // so the whole op stays `Fixed * Fixed`, instead of dropping `t` to f64 and boxing to `ops::*`.
    assert!(
        rust.contains("as i32) * 256) * speed"),
        "`t * speed` must lift the runtime integer `t` to Fixed and stay native Fixed*Fixed.\n{rust}"
    );
    // `nx = 3` — reassigning a fixed local lifts the RHS to Fixed (the E0308 reassignment fix), not f64.
    assert!(
        rust.contains("nx = tishlang_runtime::Fixed::from_raw("),
        "reassigning a `fixed` local must lift the RHS to Fixed (not leave it f64).\n{rust}"
    );
}

#[test]
fn off_gba_fixed_stays_boxed_no_native_fixed_type() {
    let path = write_src();
    // Default (desktop) emit: `fixed` has no host type, so every one of the above must stay boxed —
    // NO `tishlang_runtime::Fixed` anywhere (it would not compile off-GBA).
    let rust = compile_project_full(&path, path.parent(), &[], true).unwrap().0;
    assert!(
        !rust.contains("tishlang_runtime::Fixed"),
        "`fixed` (incl. the `t * speed` lift and `nx =` reassign) must stay boxed off-GBA.\n{rust}"
    );
}
