//! #727: VM general slot mode dropped simple-param DEFAULT values.
//!
//! `function f(a, b = 5) { let c = a + b; return c }` puts the chunk in GENERAL slot
//! mode (`let c` breaks the param-only fast path; nothing is captured, so `slot_analyze`
//! accepts). Params are slot-bound, and every READ of `b` resolves to `LoadLocal <slot>`
//! — but `emit_param_defaults_prologue` consulted only `slot_ctx` (the simple param-only
//! map, `None` here) and wrote the applied default via `DeclareVarPlain` into the
//! name-based scope map. The default was invisible: `b` read the uninitialized slot's
//! `Null`, so `f(1)` computed `1 + null` → NaN.
//!
//! These are behavior tests — they must hold identically with `TISH_VM_SLOTS=0` (the
//! name-based legacy path, correct all along) and with slots on (default).

use tishlang_bytecode::compile;

fn run_src(src: &str) {
    let program = tishlang_parser::parse(src).expect("parse");
    let chunk = compile(&program).expect("compile");
    tishlang_vm::run(&chunk).expect("run");
}

/// The issue repro: a missing arg must get its default in general slot mode.
#[test]
fn general_slot_mode_missing_arg_gets_default() {
    run_src(
        r#"
function f(a, b = 5) {
  let c = a + b
  return c
}
if (f(1) !== 6) { throw "default lost in general slot mode: f(1) = " + f(1) }
"#,
    );
}

/// A supplied arg must win over the default (and stay slot-read correct).
#[test]
fn general_slot_mode_supplied_arg_wins() {
    run_src(
        r#"
function f(a, b = 5) {
  let c = a + b
  return c
}
if (f(1, 2) !== 3) { throw "supplied arg broken: f(1, 2) = " + f(1, 2) }
"#,
    );
}

/// A default referencing an EARLIER param evaluates against that param's slot and must
/// land in this param's slot.
#[test]
fn general_slot_mode_default_references_earlier_param() {
    run_src(
        r#"
function dep(a, b = a + 1) {
  let c = a + b
  return c
}
if (dep(3) !== 7) { throw "dependent default lost: dep(3) = " + dep(3) }
if (dep(3, 10) !== 13) { throw "dependent default should not fire: dep(3, 10) = " + dep(3, 10) }
"#,
    );
}

/// A chain of dependent defaults: each later default sees the earlier ones' results.
#[test]
fn general_slot_mode_chained_dependent_defaults() {
    run_src(
        r#"
function chain(a, b = a * 2, c = b + a) {
  let out = [a, b, c]
  return out
}
let r = chain(3)
if (r[0] !== 3 || r[1] !== 6 || r[2] !== 9) { throw "chained defaults broken: " + r }
let r2 = chain(3, 10)
if (r2[1] !== 10 || r2[2] !== 13) { throw "chained defaults (partial) broken: " + r2 }
"#,
    );
}

/// A fn whose PARAM is captured is disqualified from slot mode entirely — the name-based
/// `DeclareVarPlain` path was always correct and must stay so.
#[test]
fn slot_disqualified_captured_param_default_unchanged() {
    run_src(
        r#"
function g(a, b = 5) {
  function h() { return b }
  let c = a + h()
  return c
}
if (g(1) !== 6) { throw "name-based default broken: g(1) = " + g(1) }
if (g(1, 2) !== 3) { throw "name-based supplied arg broken: g(1, 2) = " + g(1, 2) }
"#,
    );
}

/// The simple param-only fast path (`slot_ctx`) was always correct and must stay so.
#[test]
fn simple_param_only_fast_path_default_unchanged() {
    run_src(
        r#"
function f(a, b = 5) { return a + b }
if (f(1) !== 6) { throw "param-only default broken: f(1) = " + f(1) }
if (f(1, 2) !== 3) { throw "param-only supplied arg broken: f(1, 2) = " + f(1, 2) }
"#,
    );
}

/// An explicit `null` is NOT a missing arg: the default must not apply (tish has no
/// `undefined`; `ArgMissing(i)` is true iff `i >= argc`). Same in both slot modes.
#[test]
fn explicit_null_is_not_missing() {
    run_src(
        r#"
function f(a, b = 5) {
  let isNull = b === null
  return isNull
}
if (f(1, null) !== true) { throw "explicit null was replaced by the default" }
if (f(1) !== false) { throw "missing arg did not get the default" }
"#,
    );
}
