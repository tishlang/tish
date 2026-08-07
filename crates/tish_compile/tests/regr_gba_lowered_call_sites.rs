//! Regressions at the boundary between a LOWERED function and the code that calls it, both found by
//! building real GBA games (tish-gba `examples/hyrule`, `examples/ffta`) after M5 was enabled there.
//!
//! Both are compile-time breaks in the GENERATED Rust, so the assertions read the emitted source
//! rather than running anything: if the shape is wrong, rustc rejects the program and the game does
//! not build at all. Emitted through `NativeEmitMode::Gba` because that is the only mode where the
//! extended numeric vocabulary (`i32`/`fixed`) lowers natively — see #603.

use std::path::PathBuf;

use tishlang_compile::{compile_project_full_emit, NativeEmitMode};

fn compile_gba(stem: &str, src: &str) -> String {
    compile_gba_multi(stem, src, &[])
}

/// Compile `src` as the entry point, with `extra` written beside it as importable modules.
/// #622 only reproduces ACROSS a module boundary — the whole point of the bug is that the error
/// lands in a shared package the consumer never wrote — so the harness has to be able to build one.
fn compile_gba_multi(stem: &str, src: &str, extra: &[(&str, &str)]) -> String {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/regr_gba_call_sites")
        .join(stem);
    std::fs::create_dir_all(&dir).unwrap();
    for (name, body) in extra {
        std::fs::write(dir.join(name), body).unwrap();
    }
    let path = dir.join("main.tish");
    std::fs::write(&path, src).unwrap();
    let path = path.canonicalize().unwrap();
    let (rust, _, _, _) =
        compile_project_full_emit(&path, path.parent(), &[], true, NativeEmitMode::Gba, None)
            .unwrap();
    rust
}

// ── #623: a fixed-point argument reaching an f64 parameter ───────────────────────────────────────

/// `declare fn` gives the compiler a typed extern whose return is `fixed` (agb `Num<i32,8>`), which
/// is exactly what `entity_x` is in tish-gba. Passing that straight into a lowered `-> f64` helper
/// emitted the raw `Num<i32,8>` with no coercion: `error[E0308]: expected f64, found Num<i32, 8>`,
/// four times in one file of hyrule.
const FIXED_ARG: &str = r#"
function pxTile(px: number): number {
  let frac = px % 1
  let ipx = px - frac
  let m = ipx % 16
  return (ipx - m) / 16
}


let ex: fixed = 12.5
let col: i32 = pxTile(ex)
console.log(col)
"#;

#[test]
fn fixed_extern_result_coerces_at_a_lowered_call_site() {
    let rust = compile_gba("fixed_arg", FIXED_ARG);

    // Only meaningful if the helper actually lowered; otherwise the call is boxed and the bug
    // cannot arise. Guard so a future eligibility change turns this into a failure to investigate
    // rather than a silent pass.
    assert!(
        rust.contains("_native(mut px"),
        "pxTile should lower to a native fn — without that this test proves nothing:\n{}",
        rust.lines().filter(|l| l.contains("pxTile")).take(4).collect::<Vec<_>>().join("\n")
    );

    // The call site must not hand a Num<i32,8> to an f64 parameter. `ex` is `fixed`; reaching a
    // native f64 param it has to be converted, not passed raw.
    let bad = rust.lines().find(|l| {
        l.contains("_native(") && l.contains("(ex)") && !l.contains("as f64") && !l.contains("/ 256")
    });
    assert!(
        bad.is_none(),
        "a `fixed` value reaches an f64 native parameter with no coercion (#623):\n  {}",
        bad.unwrap_or("")
    );
}

// ── #622: a lowered fn's parameter name hoisted into an unrelated capture list ───────────────────

/// The shape that broke 21 tish-gba examples at once: a shared package builds an object out of
/// arrow properties whose parameter is named `v`, and ANY consumer that also has a lowered function
/// with a parameter named `v` fails with `cannot find value v` — in the package, which the consumer
/// never wrote. The lowered fn's parameters vanish as far as the module scope is concerned, and a
/// hoisted capture of the same name then has nothing to bind to.
const PKG: &str = r#"
let acc: i32 = 0
export function makeEntity(id: i32) {
  let obj = { id: id }
  obj.setVy = (v) => { acc = acc + v }
  obj.moveSpeed = (v) => { acc = acc + v * 2 }
  obj.hurt = (v) => { acc = acc - v }
  return obj
}
export function total(): i32 { return acc }
"#;

const PARAM_COLLISION: &str = r#"
import { makeEntity, total } from './pkg'

function roomBase(v: i32, size: i32): i32 { return v - (v % size) }

let e = makeEntity(1)
e.setVy(3)
e.moveSpeed(4)
console.log(roomBase(37, 16) + total())
"#;

#[test]
fn a_lowered_fn_param_is_not_hoisted_into_a_capture_list() {
    let rust = compile_gba_multi("param_collision", PARAM_COLLISION, &[("pkg.tish", PKG)]);

    // Must actually LOWER, or the collision this guards cannot arise and the test proves nothing.
    assert!(
        rust.contains("fn roomBase_native(mut v: i32"),
        "roomBase should lower with the i32 ABI — otherwise this test is vacuous:\n{}",
        rust.lines().filter(|l| l.contains("roomBase")).take(4).collect::<Vec<_>>().join("\n")
    );

    // `v` is an arrow PARAMETER and a lowered fn's parameter. Neither is a module binding, so no
    // module-level capture cell may be minted for it — that cell has nothing to clone from.
    let hoisted: Vec<&str> = rust
        .lines()
        .map(|l| l.trim())
        .filter(|l| l.starts_with("let v_cell = VmRef::new(v.clone());"))
        .collect();
    assert!(
        hoisted.is_empty(),
        "a lowered fn's parameter name was hoisted into a capture list (#622): {} site(s)\n  {}",
        hoisted.len(),
        hoisted.join("\n  ")
    );
}

/// The same defect from the other side: whatever the capture machinery decides, the emitted program
/// must not reference a binding that does not exist at that point. This is the assertion that would
/// have caught #622, #52 and #594 alike, so it is worth having as a standing shape check.
#[test]
fn no_capture_cell_references_an_undefined_binding() {
    for (stem, src, extra) in [
        ("param_collision2", PARAM_COLLISION, &[("pkg.tish", PKG)][..]),
        ("fixed_arg2", FIXED_ARG, &[][..]),
    ] {
        let rust = compile_gba_multi(stem, src, extra);
        for (i, line) in rust.lines().enumerate() {
            let t = line.trim();
            // `let X_cell = VmRef::new(X.clone());` is only sound when `X` is in scope where it is
            // emitted. At module level that means a module binding — a fn parameter never is.
            if let Some(rest) = t.strip_prefix("let ") {
                if let Some((cell, tail)) = rest.split_once("_cell = VmRef::new(") {
                    let src_name = tail.trim_end_matches(");").trim_end_matches(".clone()");
                    if src_name == cell && !line.starts_with("        ") {
                        // Module-level (shallow indent) capture of a bare name: it must be declared
                        // somewhere above as a `let`.
                        let declared = rust
                            .lines()
                            .take(i)
                            .any(|d| {
                                let d = d.trim();
                                d.starts_with(&format!("let {} ", cell))
                                    || d.starts_with(&format!("let {}:", cell))
                                    || d.starts_with(&format!("let mut {} ", cell))
                                    || d.starts_with(&format!("let {} =", cell))
                            });
                        assert!(
                            declared,
                            "{stem}: capture cell for `{cell}` at line {} references a binding that \
                             is never declared at that scope:\n  {t}",
                            i + 1
                        );
                    }
                }
            }
        }
    }
}

// ── #594: a lowered fn's body referencing a module binding it cannot see ─────────────────────────

/// Scalars in, ARRAY out — lowered to a free `fn build_nv(..) -> Vec<f64>` whose body still names
/// `CFG`, a `let` inside `run()`. `error[E0425]: cannot find value CFG in this scope`.
///
/// This is the NATIVE-VEC path, a different collector from the M5 one that `native_safe_indexable`
/// hardened, and it reads a MEMBER rather than an index — which is why hardening M5 did not cover
/// it. The fix is the same shape: a fn whose body needs module scope is not eligible to be lowered
/// to a free fn, so it stays boxed and the program compiles. Giving such a fn real access to module
/// state (emitting those bindings as statics) is the separate capability #594 also asks for.
const MODULE_SCOPE_ARRAY_OUT: &str = r#"
const CFG = { n: 3 }
function build(raceId) {
  let out = []
  let i = 0
  while (i < CFG.n) {
    out.push(raceId * 10 + i)
    i = i + 1
  }
  return out
}
const R = build(2)
console.log(R.length)
"#;

#[test]
fn a_lowered_vec_fn_never_references_module_scope() {
    let rust = compile_gba("module_scope_nv", MODULE_SCOPE_ARRAY_OUT);

    // Find every free `fn ..._nv(` body and prove none of them names `CFG`. A free fn is emitted at
    // top level; `CFG` is a local of `run()`; the two cannot see each other.
    let mut in_nv = false;
    let mut depth = 0i32;
    let mut offenders: Vec<String> = Vec::new();
    for line in rust.lines() {
        if !in_nv && line.trim_start().starts_with("fn ") && line.contains("_nv(") {
            in_nv = true;
            depth = 0;
        }
        if in_nv {
            depth += line.matches('{').count() as i32 - line.matches('}').count() as i32;
            if line.contains("CFG") && !line.trim_start().starts_with("fn ") {
                offenders.push(line.trim().to_string());
            }
            if depth <= 0 && line.contains('}') {
                in_nv = false;
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "a lowered `_nv` free fn references module-scope `CFG`, which does not exist at top level \
         (#594) — {} line(s):\n  {}",
        offenders.len(),
        offenders.join("\n  ")
    );
}

// ── #605: VecRetKind has no String, so a string-returning fn cannot lower ────────────────────────

/// Scalars in, STRING out. Correct today but boxed: the fn becomes a `Value::native` closure in a
/// `VmRef` cell and every call is a `value_call`. `VecRetKind` only spoke `{F64, VecF64, Unit}`, so
/// there was no shape for it to lower into.
///
/// Every return here is a string literal — the same standard the numeric path holds itself to
/// (`vec_fn_return_kind` proves EVERY return agrees on one shape, never trusting the annotation,
/// because a return type is erased and unchecked: the `liar()` case).
const STRING_OUT: &str = r#"
function label(n: i32): string {
  if (n === 0) { return "zero" }
  if (n === 1) { return "one" }
  return "many"
}
let s: string = label(2)
console.log(s)
"#;

#[test]
fn a_string_returning_fn_lowers_to_a_native_fn() {
    let rust = compile_gba("string_out", STRING_OUT);
    assert!(
        rust.contains("fn label_nv(") && rust.contains("-> String"),
        "a scalars-in/string-out fn should lower to a native `-> String` fn (#605), got:\n{}",
        rust.lines().filter(|l| l.contains("label")).take(5).collect::<Vec<_>>().join("\n")
    );
}

/// The guard that keeps #605 sound. A fn whose returns do NOT all agree on `String` must stay
/// boxed — mixing a string and a number is exactly the shape a single native signature cannot
/// express, and guessing costs a miscompile rather than a slow path.
const MIXED_OUT: &str = r#"
function maybe(n: i32) {
  if (n === 0) { return "zero" }
  return n * 2
}
console.log(maybe(1))
console.log(maybe(0))
"#;

#[test]
fn a_mixed_string_number_return_stays_boxed() {
    let rust = compile_gba("mixed_out", MIXED_OUT);
    assert!(
        !rust.contains("fn maybe_nv("),
        "a fn returning both a string and a number must NOT lower (#605 soundness):\n{}",
        rust.lines().filter(|l| l.contains("maybe_nv")).take(3).collect::<Vec<_>>().join("\n")
    );
}

// ── #594 (the capability): module state a lowered fn CAN see, on GBA ─────────────────────────────

/// A module const numeric array read from a lowered fn. `const G_TABLE: [f64; N] = [..]` needs no
/// std at all — it was only ever unavailable on GBA because its pass shared a gate with the
/// `thread_local!` one.
const CONST_ARRAY_READ: &str = r#"
const TABLE = [3, 1, 4, 1, 5, 9, 2, 6]
function pick(n) {
  let out = []
  let i = 0
  while (i < n) {
    out.push(TABLE[i] * 2)
    i = i + 1
  }
  return out
}
const R = pick(4)
console.log(R.length)
"#;

#[test]
fn a_module_const_array_is_reachable_from_a_lowered_fn_on_gba() {
    let rust = compile_gba("const_array_gba", CONST_ARRAY_READ);
    assert!(
        rust.contains("const G_TABLE: [f64; 8]") || rust.contains("const G_table: [f64; 8]"),
        "a module const numeric array should get a top-level `const` on GBA (#594):\n{}",
        rust.lines().filter(|l| l.contains("TABLE")).take(4).collect::<Vec<_>>().join("\n")
    );
    assert!(
        rust.contains("fn pick_nv("),
        "and the fn that reads it should lower (#594) — that is the whole point:\n{}",
        rust.lines().filter(|l| l.contains("pick")).take(4).collect::<Vec<_>>().join("\n")
    );
}

/// A mutable module numeric global read AND written from a lowered fn. On GBA the storage is the
/// facade's `SingleCore<Cell<f64>>`, whose `with()` deliberately mirrors `LocalKey::with` — so the
/// access shape the emitter already produces works unchanged.
const NUMERIC_GLOBAL: &str = r#"
let seed = 7
function nextBase(): number {
  seed = (seed * 3877 + 29573) % 139968
  if (seed < 34992) { return 0 }
  return 3
}
let acc = 0
let k = 0
while (k < 10) { acc = acc + nextBase(); k = k + 1 }
console.log(acc)
"#;

#[test]
fn a_module_numeric_global_is_reachable_from_a_lowered_fn_on_gba() {
    let rust = compile_gba("numeric_global_gba", NUMERIC_GLOBAL);
    // No `thread_local!` may reach a no_std target.
    assert!(
        !rust.contains("thread_local!"),
        "GBA output must not contain thread_local! (no_std):\n{}",
        rust.lines().filter(|l| l.contains("thread_local")).take(3).collect::<Vec<_>>().join("\n")
    );
    assert!(
        rust.contains("SingleCore") && rust.contains("Cell<f64>"),
        "a mutable module numeric global should live in a SingleCore<Cell<f64>> static on GBA \
         (#594):\n{}",
        rust.lines().filter(|l| l.contains("seed") || l.contains("SingleCore")).take(5)
            .collect::<Vec<_>>().join("\n")
    );
    assert!(
        rust.contains("fn nextBase_native("),
        "and the fn that touches it should lower (#594):\n{}",
        rust.lines().filter(|l| l.contains("nextBase")).take(4).collect::<Vec<_>>().join("\n")
    );
}

/// The Fixed conversions must agree with ToInt32 on SIGN, not just magnitude. An arithmetic shift
/// (`>> 8`) floors, so a negative fixed would land one below where `as i32` puts it — -1.5 to -2
/// rather than -1. Non-negative values agree either way, which is how such a bug ships.
#[test]
fn fixed_to_int_truncates_toward_zero_not_floor() {
    let rust = compile_gba(
        "fixed_trunc",
        r#"
declare fn take(n: i32): i32
let f: fixed = 0 - 1.5
let g: i32 = 7
console.log(g)
"#,
    );
    assert!(
        !rust.contains(".to_raw() >> 8"),
        "a Fixed->i32 conversion must divide (truncate toward zero), never arithmetic-shift (floor)"
    );
}

// ── Findings from the adversarial review of the changes above ────────────────────────────────────

/// A string-returning fn that can FALL OFF THE END must not lower. `vec_fn_return_kind` rejects an
/// explicit bare `return;` but cannot see an implicit fall-through, and the lowered tail yields
/// `String::new()` where the boxed path yields `Value::Null` — so `label(-1)` returned "" instead of
/// null, silently, from ordinary code. `label(-1) === null` flipped true to false.
#[test]
fn a_partial_string_fn_does_not_lower() {
    let rust = compile_gba(
        "partial_string",
        r#"
function label(n) {
  if (n > 0) { return "pos" }
}
console.log(label(-1))
"#,
    );
    assert!(
        !rust.contains("fn label_nv("),
        "a string fn with an implicit fall-through must stay boxed — its lowered tail returns \"\" \
         where tish returns null:\n{}",
        rust.lines().filter(|l| l.contains("label")).take(3).collect::<Vec<_>>().join("\n")
    );
}

/// `===` is OVERLOADED: it compares strings as readily as numbers, so it is not proof that an
/// operand is numeric. Treating it as arithmetic handed a string parameter the f64 ABI, which is a
/// `panic!("expected number")` on a no_std cart.
#[test]
fn a_string_compared_param_does_not_get_the_f64_abi() {
    let rust = compile_gba(
        "string_param",
        r#"
function suit(tag) {
  if (tag === "fire") { return "hearts" }
  return "spades"
}
console.log(suit("fire"))
"#,
    );
    assert!(
        !rust.contains("fn suit_nv(mut tag: f64"),
        "a param proven 'numeric' only by `=== \"fire\"` must not take the f64 ABI:\n{}",
        rust.lines().filter(|l| l.contains("suit")).take(3).collect::<Vec<_>>().join("\n")
    );
}

/// A module const array must be disqualified by ANY binding of its name — a parameter, a for-of
/// variable, a catch binding — not just a `let`. The const fast path in the Index arm is static with
/// no scope check, so the module array was read in place of the local one: a WRONG VALUE with no
/// diagnostic, on every target.
#[test]
fn a_param_shadowing_a_module_const_array_is_not_the_module_one() {
    let rust = compile_gba(
        "shadow_const",
        r#"
const T = [1, 2, 3]
function f(T, i) { return T[i] * 2 }
console.log(f([9, 8, 7], 0))
"#,
    );
    assert!(
        !rust.contains("const G_T:"),
        "a name bound as a parameter anywhere must not become a module const (wrong value):\n{}",
        rust.lines().filter(|l| l.contains("G_T")).take(3).collect::<Vec<_>>().join("\n")
    );
}

/// `let a = 1, b = 2` parses to `Statement::Multi`, and codegen emits each declarator into the
/// current scope — at module level, a `run()` local. The module-binding guard must see them, or a
/// free fn names one and the crate fails with E0425.
#[test]
fn multi_declared_module_bindings_are_seen_by_the_guard() {
    let rust = compile_gba(
        "multi_binding",
        r#"
const CFG = { n: 3 }, STEP = 10
function build(raceId) {
  let out = []
  let i = 0
  while (i < CFG.n) { out.push(raceId * STEP + i); i = i + 1 }
  return out
}
const R = build(2)
console.log(R.length)
"#,
    );
    let mut in_nv = false;
    let mut offenders: Vec<String> = Vec::new();
    for line in rust.lines() {
        if line.trim_start().starts_with("fn ") && line.contains("_nv(") {
            in_nv = true;
        } else if in_nv && line.starts_with('}') {
            in_nv = false;
        } else if in_nv && (line.contains("CFG") || line.contains("STEP")) {
            offenders.push(line.trim().to_string());
        }
    }
    assert!(
        offenders.is_empty(),
        "a Multi-declared module binding leaked into a lowered fn (E0425):\n  {}",
        offenders.join("\n  ")
    );
}

// ── #48/#52-follow-up: the module-const-array hoist was blind to closures ─────────────────────────
//
// `collect_module_const_f64_arrays` hoists a module-level numeric array to a top-level
// `const G_X: [f64; N]` so a LOWERED fn can reach it (that reachability is the whole point of #594).
// Admission is supposed to reject any array that is written, or used as a value/member, because a
// `const` is neither mutable nor addressable. The admission walker checks all of that — but its
// expression arm had no `ArrowFunction` case, so every use inside a CLOSURE fell into `_ => {}` and
// was invisible. The sibling scalar pass (`walk_expr_for_global_disqualifiers`) has always had that
// arm; this walker is a copy that lost it.
//
// Net effect once the pass was enabled on GBA: an array that a closure writes got hoisted anyway.
// The read side renamed to `G_X`, the write side kept the boxed local, and the boxed local is
// declared *after* the closure — so rustc reports `cannot find value` on a name the game never
// misspelled. Reported against card-gba (queensblood/triad/gwent) and reproduced independently by
// tish-gba `examples/hyrule` (`rangLive`, weapons.tish).
//
// The trigger needs all three ingredients: a module numeric array, a typed top-level fn that reads
// it (that is what nominates it for hoisting), and a write from inside a closure.

/// The reported repro, reduced. Before the fix this emitted `const G_A: [f64; 3]` plus a write
/// through a bare `A`, and rustc rejected it with E0425.
const CLOSURE_WRITE: &str = r#"
let A: i32[] = [1, 2, 3]

function readA(i: i32): i32 { return A[i] }

export function run(): i32 {
  let bump = () => { A[0] = A[0] + 1 }
  bump()
  return readA(0)
}
"#;

#[test]
fn module_array_written_in_closure_is_not_hoisted_to_const() {
    let rust = compile_gba("closure_write", CLOSURE_WRITE);
    // The array is written, so it must stay a runtime binding: no const hoist at all. Renaming the
    // write site would not be enough — a `const` cannot be assigned through.
    assert!(
        !rust.contains("const G_A"),
        "a module array written from a closure must not be hoisted to a const:\n{}",
        rust
    );
    // And the generated program must not reference a hoisted name it never declared.
    assert!(
        !rust.contains("G_A["),
        "no read may be renamed to a hoist that does not exist:\n{}",
        rust
    );
}

/// The second missed case from the same report: `.length` lowers to `.len()`, which is a member use
/// the walker already rejects — it was just unreachable inside a closure. A hoist here emitted
/// `A.len()` against a name not in scope.
const CLOSURE_LENGTH: &str = r#"
let A: i32[] = [1, 2, 3]

function readA(i: i32): i32 { return A[i] }

export function run(): i32 {
  let count = () => { return A.length }
  return readA(0) + count()
}
"#;

#[test]
fn module_array_member_used_in_closure_is_not_hoisted_to_const() {
    let rust = compile_gba("closure_length", CLOSURE_LENGTH);
    assert!(
        !rust.contains("const G_A"),
        "a module array whose `.length` a closure reads must not be hoisted:\n{}",
        rust
    );
}

/// The win must survive: a read-only array indexed from a closure is still a legal hoist, because
/// the Index arm deliberately does not count `X[i]` as a disqualifying use of `X`. Without this the
/// fix would be a blanket "any closure mention disables hoisting" and would give back #594.
const CLOSURE_READ_ONLY: &str = r#"
let A: i32[] = [1, 2, 3]

function readA(i: i32): i32 { return A[i] }

export function run(): i32 {
  let pick = () => { return A[1] }
  return readA(0) + pick()
}
"#;

#[test]
fn read_only_module_array_still_hoists_when_a_closure_indexes_it() {
    let rust = compile_gba("closure_read_only", CLOSURE_READ_ONLY);
    assert!(
        rust.contains("G_A"),
        "a read-only module array must still hoist — that reachability is #594:\n{}",
        rust
    );
}

/// Not a gate — a probe that prints the emitted representation of a hoisted `i32[]` on GBA, so the
/// f64-on-a-CPU-with-no-FPU question can be answered from generated code rather than assumption.
#[test]
#[ignore]
fn probe_hoisted_array_representation() {
    let rust = compile_gba("probe_repr", CLOSURE_READ_ONLY);
    for line in rust.lines() {
        if line.contains("G_A") {
            println!("{}", line.trim());
        }
    }
}

/// The SECOND hole, found only because the first fix left queensblood at 4 errors instead of 12: the
/// statement walker visited loop and branch BODIES but not their CONDITIONS. `.length` is a member
/// use the walker already rejects — it was simply never reached in `while (i < X.length)`, which is
/// how card-gba's `SB_FORCE`/`SB_PRE`/`TN_REWARD` (exported `i32[]` in qb-catalog.tish, consumed by
/// qb-engine.tish:567 and main.tish:297) stayed eligible and hoisted.
const COND_LENGTH: &str = r#"
let A: i32[] = [1, 2, 3]

function readA(i: i32): i32 { return A[i] }

export function run(): i32 {
  let i: i32 = 0
  let n: i32 = 0
  while (i < A.length) { n = n + readA(i); i = i + 1 }
  return n
}
"#;

#[test]
fn a_use_in_a_loop_condition_disqualifies_the_hoist() {
    let rust = compile_gba("cond_length", COND_LENGTH);
    assert!(
        !rust.contains("const G_A"),
        "`.length` in a while-condition is a member use and must block the hoist:\n{}",
        rust
    );
}

/// Same shape, in an `if` condition — that arm had the identical omission.
const IF_COND: &str = r#"
let A: i32[] = [1, 2, 3]

function readA(i: i32): i32 { return A[i] }

export function run(): i32 {
  if (A.length > 2) { return readA(0) }
  return 0
}
"#;

#[test]
fn a_use_in_an_if_condition_disqualifies_the_hoist() {
    let rust = compile_gba("if_cond", IF_COND);
    assert!(
        !rust.contains("const G_A"),
        "`.length` in an if-condition must block the hoist:\n{}",
        rust
    );
}

// ── #594 residual, reported from tish-gba examples/hyrule ─────────────────────────────────────────
//
// Reported shape: a fn with NO PARAMS and NO RETURN that WRITES a module array, and IS CALLED. On
// 3.3.0 that emitted the new `vm_read(&CELL)` module-scope capability without bringing the cell into
// scope. Two facts from the report narrow it and are both encoded below:
//   - the identical shape with NO callers compiles, because it is dead-code eliminated, so a repro
//     MUST contain a call site;
//   - inlining the body at the call site is a complete workaround.
//
// This is the `rangDone()` shape in hyrule's weapons.tish (`export function rangDone() { rangLive[0] = 0 }`),
// which is the same `rangLive` E0425 that started this investigation.

/// No params, no return, writes a module array, and — critically — is called.
const WRITER_WITH_CALLER: &str = r#"
let A: i32[] = [0, 0, 0]

function readA(i: i32): i32 { return A[i] }

function clearA() { A[0] = 0 }

export function run(): i32 {
  clearA()
  return readA(0)
}
"#;

#[test]
fn a_called_no_param_no_return_module_array_writer_compiles() {
    let rust = compile_gba("writer_with_caller", WRITER_WITH_CALLER);
    // The array is written, so it must NOT have been hoisted…
    assert!(
        !rust.contains("const G_A"),
        "a written module array must not be hoisted:\n{}",
        rust
    );
    // …and no lowered free fn may reference the boxed cell, which lives in `run()`. A free `fn`
    // mentioning `A` is precisely the E0425 the report describes.
    for (i, line) in rust.lines().enumerate() {
        let t = line.trim_start();
        if t.starts_with("fn ") || t.starts_with("pub fn ") {
            assert!(
                !t.contains("clearA") || t.contains("_cell"),
                "clearA must not be emitted as a free fn that cannot see module scope (line {}): {}",
                i + 1,
                t
            );
        }
    }
}
