//! Generated-Rust assertions for the #174 Int32/U32 range-lattice round-trip erasure.
//!
//! #174 erases redundant `tishlang_runtime::to_int32` / `to_uint32` calls in the typed bitwise/shift
//! emitter when an operand is PROVABLY a finite integer — either an integer literal (a compile-time
//! constant) or a value the integer-range lattice bounds within `(-2^53, 2^53)`. The guarded
//! `to_int32` (NaN/±Infinity → 0) is retained for every unproven leaf, so the lowering is purely
//! additive and behaviour-identical (the gauntlet's `typed == boxed == node` checksum is the gate).
//!
//! These run in their own test binary so any env flags set here never leak into other tests.

use tishlang_compile::compile;
use tishlang_parser::parse;

fn enable_typed_flags() {
}

/// FNV-style hash loop: the canonical bitwise hot loop. The accumulator `h` lives in an i32 register
/// (existing lowering); #174 additionally folds the constant mask / shift counts and keeps the
/// `(h * C) >>> 0` f64-rounding excursion intact.
const FNV: &str = r#"
let h = 2166136261
for (let i = 0; i < 100; i++) {
  h = h ^ (i & 255)
  h = (h * 16777619) >>> 0
  h = ((h << 13) | (h >>> 19)) >>> 0
}
let check = h >>> 0
console.log(check)
"#;

#[test]
fn issue_174_constant_mask_and_shift_counts_fold() {
    // `i & 255`, `h << 13`, `h >>> 19`, `… >>> 0`: every integer-literal operand is a compile-time
    // ToInt32 constant, so it must emit as a bare `i32` literal — NOT a runtime `to_int32(255_f64)`.
    let rust = compile(&parse(FNV).unwrap()).unwrap();
    assert!(
        rust.contains("255i32"),
        "the `& 255` mask must fold to an i32 constant:\n{}",
        rust.lines().filter(|l| l.contains("255")).take(4).collect::<Vec<_>>().join("\n")
    );
    assert!(
        rust.contains("13i32") && rust.contains("19i32"),
        "shift counts 13 / 19 must fold to i32 constants"
    );
    // No runtime ToInt32/ToUint32 call on an integer LITERAL remains.
    assert!(
        !rust.contains("to_int32(255") && !rust.contains("to_uint32(13")
            && !rust.contains("to_uint32(19") && !rust.contains("to_uint32(0"),
        "integer-literal operands must not round-trip through a runtime to_int32/to_uint32 call:\n{}",
        rust.lines().filter(|l| l.contains("to_uint32(") || l.contains("to_int32(")).take(8).collect::<Vec<_>>().join("\n")
    );
}

#[test]
fn issue_174_fnv_accumulator_still_i32_register_no_regression() {
    // The i32-register accumulator lowering (and its `(h * C)` f64-rounding excursion) is preserved:
    // `h` is an i32 register, there is no per-op `to_int32(h)` round-trip, and the multiply excursion
    // lowers to an unchecked f64 truncation. (Mirror of `fnv_accumulator_lowers_to_i32_register`.)
    let rust = compile(&parse(FNV).unwrap()).unwrap();
    assert!(
        rust.contains("let mut h: i32 = (2166136261u32) as i32;"),
        "h must stay an i32 register seeded via u32 reinterpretation"
    );
    assert!(
        !rust.contains("to_int32(h)"),
        "no per-op to_int32(h) round-trip on the i32 accumulator"
    );
    assert!(
        rust.contains(".to_int_unchecked::<i64>()") && rust.contains("16777619"),
        "the `h * 16777619` excursion stays an f64 multiply with an unchecked truncation"
    );
}

#[test]
fn issue_174_range_proven_counter_drops_is_finite_guard() {
    // A loop counter masked into a bitwise op is range-proven integral, so its ToInt32 must not
    // round-trip through the guarded `to_int32(i)` call. The guard-free lowering is the saturating
    // `(i) as i64 as i32` cast — #380 (PR #387) replaced the original `to_int_unchecked::<i64>()`
    // here because `int_valued_locals` proves integrality but not magnitude, making the unchecked
    // truncation UB on an overflowing accumulator; the saturating cast is defined for all f64 at
    // the same hot-path cost (fnv_hash/fannkuch/mandelbrot measured at baseline in #387).
    let src = r#"
let acc = 0
for (let i = 0; i < 64; i = i + 1) {
  acc = acc ^ (i & 15)
}
console.log(acc >>> 0)
"#;
    let rust = compile(&parse(src).unwrap()).unwrap();
    assert!(
        rust.contains("as i64 as i32"),
        "a range-proven counter `i` in `i & 15` must lower to the guard-free saturating cast:\n{}",
        rust.lines().filter(|l| l.contains("acc") || l.contains("to_int")).take(8).collect::<Vec<_>>().join("\n")
    );
    assert!(
        !rust.contains("to_int32(i)"),
        "a range-proven counter must not round-trip through the guarded to_int32 call:\n{}",
        rust.lines().filter(|l| l.contains("to_int32(")).take(8).collect::<Vec<_>>().join("\n")
    );
    // Nothing in this program is the bounded `(h * C) >>> 0` excursion shape, so the UB-prone
    // unchecked truncation must not appear at all (#380).
    assert!(
        !rust.contains("to_int_unchecked"),
        "no unchecked truncation may appear for an unbounded-magnitude program"
    );
}

#[test]
fn fnv_value_fn_uses_usize_for_loop() {
    let src = r#"
function fnv1a(n) {
  let h = 2166136261
  for (let i = 0; i < n; i++) {
    h = h ^ (i & 255)
    h = (h * 16777619) >>> 0
    h = ((h << 13) | (h >>> 19)) >>> 0
  }
  return h >>> 0
}
let check = fnv1a(100)
console.log(check)
"#;
    enable_typed_flags();
    let rust = compile(&parse(src).unwrap()).unwrap();
    assert!(
        rust.contains("for _usize_i_0 in 0..(n as usize)"),
        "fnv1a hot loop should lower to a usize `for` over param n:\n{}",
        rust.lines().filter(|l| l.contains("usize") || l.contains("fnv1a")).take(6).collect::<Vec<_>>().join("\n")
    );
    assert!(
        rust.contains("let mut h: i32 = (2166136261u32) as i32;"),
        "accumulator h must stay on the i32 register path"
    );
}

#[test]
fn issue_174_unproven_leaf_keeps_guarded_to_int32() {
    // SOUNDNESS: an f64 value that is NOT provably a finite integer (here a fractional accumulator —
    // the range lattice drops it because `n = n + 0.5` is not integer-preserving) must KEEP the
    // guarded `tishlang_runtime::to_int32`, never the unchecked truncation (a non-integer/NaN/±Inf
    // input must map through the JS ToInt32 path, not a raw `to_int_unchecked`).
    let src = r#"
let n = 0.0
for (let i = 0; i < 10; i = i + 1) { n = n + 0.5 }
let m = n & 7
console.log(m >>> 0)
"#;
    let rust = compile(&parse(src).unwrap()).unwrap();
    // `n` is a top-level numeric used only at top level, so per #313 it is a native `let mut n: f64`
    // local (not a `thread_local Cell` — no function references it). The soundness invariant is
    // unchanged: the unproven fractional value must STILL pass through the guarded `to_int32` (not an
    // unchecked truncation) before the `& 7` mask.
    assert!(
        rust.contains("tishlang_runtime::to_int32(n) & 7i32"),
        "an unproven f64 leaf must keep the guarded to_int32 before the mask:\n{}",
        rust.lines().filter(|l| l.contains("& ") || l.contains("to_int")).take(6).collect::<Vec<_>>().join("\n")
    );
    // The literal `7` still folds; only the unproven `n` keeps the guard.
    assert!(rust.contains("7i32"), "the literal mask `7` still folds to a constant");
}
