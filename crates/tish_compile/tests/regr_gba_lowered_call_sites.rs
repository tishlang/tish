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
