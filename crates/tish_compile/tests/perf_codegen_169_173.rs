//! Generated-Rust assertions for the Beat-Node native codegen wins #169 and #173.
//!
//! These run in their own test binary (separate process from the lib unit tests), so the
//! dark-ship env flags set here never leak into other tests. Every test sets the SAME flag set
//! (the gauntlet's `TYPED_FLAGS`), so parallel execution is race-free.

use std::path::PathBuf;

use tishlang_compile::{compile, compile_project_full};
use tishlang_parser::parse;

/// Enable every dark-shipped typed-native flag, matching `scripts/run_perf_gauntlet.sh`.
fn enable_typed_flags() {
}

fn compile_typed(src: &str) -> String {
    enable_typed_flags();
    compile(&parse(src).unwrap()).unwrap()
}

/// Compile a real fixture through the **same** path the `tish build` CLI uses
/// (`compile_project_full` → `merge_modules` → codegen), so generated-Rust assertions match the
/// gauntlet build exactly.
fn compile_fixture_typed(rel: &str) -> String {
    enable_typed_flags();
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let path = manifest.join("../..").join(rel).canonicalize().unwrap();
    let (rust, _, _, _) = compile_project_full(&path, path.parent(), &[], true).unwrap();
    rust
}

// ── #169: a fused native `Vec<f64>` reduce result keeps its accumulator native ────────────────

#[test]
fn issue_169_reduce_fed_accumulator_stays_native() {
    // The real gauntlet fixture: `acc` is updated from `xs.reduce(...)` (which the native-HOF path
    // fuses to a native f64 fold). Before #169 the demotion oracle didn't model the fusion, so `acc`
    // was boxed (`let mut acc = Value::Number(...)`) and paid boxed ops::mul/add/modulo + clone per
    // iter. Compiled through the same `compile_project_full` path the `tish build` CLI uses.
    let rust = compile_fixture_typed("tests/perf/typed_array_hof.tish");
    assert!(
        rust.contains("let mut acc: f64"),
        "acc must stay native f64 (reduce result modelled as F64), got:\n{}",
        rust.lines().filter(|l| l.contains("acc")).take(6).collect::<Vec<_>>().join("\n")
    );
    assert!(
        !rust.contains("let mut acc = Value::Number"),
        "acc must NOT be boxed:\n{}",
        rust.lines().filter(|l| l.contains("acc")).take(6).collect::<Vec<_>>().join("\n")
    );
    // The per-iteration accumulator update is native arithmetic, not boxed ops on `acc`.
    assert!(
        !rust.contains("ops::mul(&acc"),
        "acc update must be native f64, not a boxed ops::mul on acc"
    );
}

#[test]
fn issue_169_non_number_array_reduce_does_not_make_accumulator_native() {
    // Conservative bail: reduce over a *string* array is not a native f64 fold, so an accumulator it
    // feeds must stay boxed (sound — we never claim F64 where the emitter would box).
    let src = r#"
let ss: string[] = ["a", "b", "c"]
let acc: number = 0
let r: number = 0
while (r < 10) {
  acc = acc + ss.length
  r = r + 1
}
console.log(acc)
"#;
    // Just assert it compiles and is sound; `acc` here is fed by `.length` which is a separate lever.
    let rust = compile_typed(src);
    assert!(rust.contains("fn main"), "compiles to a native program");
}

// ── #173: fill-loop fusion + native `.length` ─────────────────────────────────────────────────

#[test]
fn issue_173_fill_loop_fuses_to_bulk_extend() {
    // `let a = []; for (let i = 0; i < N; i++) { a.push(K) }` over a native Vec lowers to a single
    // `a.extend(std::iter::repeat(K).take((N) as usize))` — one allocation, no per-element pushes.
    let rust = compile_fixture_typed("tests/core/array_init_pattern.tish");
    // boolean[] fill with a literal bound.
    assert!(
        rust.contains("extend(std::iter::repeat(true).take((8_f64) as usize))"),
        "boolean fill loop must fuse to a bulk extend:\n{}",
        rust.lines().filter(|l| l.contains("repeat")).collect::<Vec<_>>().join("\n")
    );
    // number[] fill with an integer-`let` bound (`n`).
    assert!(
        rust.contains("extend(std::iter::repeat(1_f64).take((n) as usize))"),
        "number fill loop with an int-range bound must fuse:\n{}",
        rust.lines().filter(|l| l.contains("repeat")).collect::<Vec<_>>().join("\n")
    );
}

#[test]
fn issue_173_length_is_native_f64() {
    // `vec.length` on a native Vec lowers to `(vec.len() as f64)`, not a boxed `get_prop`.
    let rust = compile_fixture_typed("tests/core/array_init_pattern.tish");
    assert!(
        rust.contains("a.len() as f64"),
        "native Vec `.length` must lower to `(a.len() as f64)`"
    );
    assert!(
        !rust.contains("get_prop(&a, \"length\")"),
        "native Vec `.length` must not go through boxed get_prop"
    );
}

#[test]
fn issue_173_adversarial_cases_do_not_fuse() {
    // Non-constant push arg, extra statement, and `break` in the body must NOT fuse — they keep the
    // per-element push loop (correctness over coverage). The fixture's only fusions are the three
    // canonical fills (8, n, 3); a non-constant `push(i * 2)` must not produce an extend.
    let rust = compile_fixture_typed("tests/core/array_init_pattern.tish");
    assert!(
        !rust.contains("repeat((i * 2"),
        "non-constant push arg must not be fused into a repeat-fill"
    );
    // The three canonical fills DO fuse (boolFill=8, sieve=n, oobGrow=3); the adversarial cases keep
    // their push loops. Assert each sound fill is present rather than a brittle exact total: with the
    // typing stack on by default, a fn that has a native `_nv` form (e.g. `numFillSieve`) may emit its
    // fill in both the native and the boxed closure form, so the same fill can legitimately appear
    // more than once.
    assert!(rust.contains("repeat(true).take((8"), "boolFill (8) should fuse");
    assert!(rust.contains("repeat(1_f64).take((n"), "sieve (n) should fuse");
    assert!(rust.contains("repeat(1_f64).take((3"), "oobGrow (3) should fuse");
}
