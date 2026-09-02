//! #716 follow-up: functions whose PARAMS are captured by a nested closure were
//! disqualified from general slot mode entirely (`slot_analyze` returned `None`), so the
//! per-call frame-scope↔closure cycle from #716 persisted for them — the issue's exact
//! repro (`handleEvent(payload)` with a body helper reading `payload`) still leaked after
//! #728.
//!
//! The fix is a captured-param COPY-IN PROLOGUE: params land in slots 0..n as the VM's
//! calling convention requires, and each captured param is copied into `local_scope` at
//! entry (one `LoadLocal` + `DeclareVarPlain`). From then on the SCOPE is the single
//! source of truth for that param — every body read/write compiles name-based
//! (`LoadVar`/`StoreVar`), the very same map closures capture — so shared-cell mutation
//! semantics are preserved exactly. The chunk stays in slot mode, and #728's slot-binding
//! of the (uncaptured) helper NAME applies: no cycle, the frame drops on return.

use std::sync::Arc;
use tishlang_bytecode::compile;
use tishlang_core::{Value, VmRef};
use tishlang_vm::Vm;

fn run_src(src: &str) {
    let program = tishlang_parser::parse(src).expect("parse");
    let chunk = compile(&program).expect("compile");
    tishlang_vm::run(&chunk).expect("run");
}

/// These tests pin slot-mode behavior; a suite run with `TISH_VM_SLOTS=0` deliberately
/// exercises the legacy name-based path (which still has the #716 cycle).
fn slots_disabled() -> bool {
    std::env::var("TISH_VM_SLOTS")
        .map(|v| v == "0")
        .unwrap_or(false)
}

/// The leak regression — the #716 repro shape verbatim (helper captures the enclosing
/// PARAM): N calls must not retain the N frames. The probe is the payload argument, so
/// each leaked frame would hold one strong ref via its `local_scope` (`payload` →
/// probe). Pre-fix the chunk is name-based and `fmt`'s `DeclareVar` completes the
/// scope↔closure cycle: `VmRef::strong_count(&probe)` ends at `2 + N`.
#[test]
fn captured_param_frames_are_reclaimed() {
    if slots_disabled() {
        return;
    }
    let probe: VmRef<Vec<Value>> = VmRef::new(vec![Value::Number(1.0)]);
    let mut vm = Vm::new();
    vm.set_global(Arc::from("probe"), Value::Array(probe.clone()));
    let src = r#"
function handleEvent(payload) {
  function fmt() { return payload.length }
  return fmt()
}
let i = 0
let acc = 0
while (i < 100) { acc = acc + handleEvent(probe); i = i + 1 }
if (acc !== 100) { throw "bad acc: " + acc }
"#;
    let program = tishlang_parser::parse(src).expect("parse");
    let chunk = compile(&program).expect("compile");
    vm.run(&chunk).expect("run");
    // Exactly two live references: this test's `probe` clone + the VM globals entry.
    assert_eq!(
        VmRef::strong_count(&probe),
        2,
        "call frames retained the probe — the captured-param variant of the #716 \
         frame-scope↔closure cycle is still leaking"
    );
}

/// MUTATION SEMANTICS, direction 1: the body reassigns the param, then a closure reads
/// it. The scope must be the single source of truth (a stale copy would return 1).
#[test]
fn closure_reads_param_after_body_reassigns() {
    run_src(
        r#"
function outer(p) {
  function get() { return p }
  p = p + 1
  let r = get()
  return r
}
if (outer(1) !== 2) { throw "closure saw a stale param copy: " + outer(1) }
"#,
    );
}

/// MUTATION SEMANTICS, direction 2: a closure reassigns the param (via a call), then the
/// body reads it. The body read must route to the shared scope (a slot read would see 1).
#[test]
fn body_reads_param_after_closure_reassigns() {
    run_src(
        r#"
function outer(p) {
  function bump() { p = p + 10 }
  bump()
  let r = p
  return r
}
if (outer(1) !== 11) { throw "body saw a stale param slot: " + outer(1) }
"#,
    );
}

/// Both directions interleaved, plus a second UNCAPTURED param that must stay slotted.
#[test]
fn interleaved_mutation_and_mixed_params() {
    run_src(
        r#"
function outer(p, q) {
  function bump() { p = p + 1 }
  p = p * 2
  bump()
  let r = p + q
  return r
}
if (outer(3, 10) !== 17) { throw "interleaved captured-param mutation broken: " + outer(3, 10) }
"#,
    );
}

/// An ESCAPING closure keeps the captured param (including a body reassignment made
/// before the escape) alive after the frame returns.
#[test]
fn escaping_closure_keeps_reassigned_param() {
    run_src(
        r#"
function mk(p) {
  function get() { return p }
  p = p * 2
  return get
}
let g = mk(4)
if (g() !== 8) { throw "escaping closure lost the reassigned param: " + g() }
"#,
    );
}

/// A DEFAULT on a captured param: the copy-in must run before the defaults prologue so
/// the applied default (stored name-based) is not clobbered by the missing-arg null.
#[test]
fn captured_param_default_applies() {
    run_src(
        r#"
function withDefault(a, p = 5) {
  function get() { return p }
  let r = a + get()
  return r
}
if (withDefault(1) !== 6) { throw "captured-param default lost: " + withDefault(1) }
if (withDefault(1, 2) !== 3) { throw "captured-param supplied arg broken: " + withDefault(1, 2) }
"#,
    );
}

/// Recursion: every level gets a fresh frame scope; each closure sees its own level's
/// param.
#[test]
fn recursion_with_captured_param() {
    run_src(
        r#"
function recur(n) {
  function dec() { return n - 1 }
  if (n === 0) { return 0 }
  let x = recur(dec())
  return x + n
}
if (recur(3) !== 6) { throw "recursion with captured param broken: " + recur(3) }
"#,
    );
}

/// A MISSING captured param reads null — matching the interpreter (and slot mode's
/// existing missing-arg behavior). The legacy name-based VM chunk left the name unbound
/// ("Undefined variable"), a pre-existing interp divergence this shape no longer hits.
#[test]
fn missing_captured_param_is_null() {
    if slots_disabled() {
        return;
    }
    run_src(
        r#"
function m(a, b) {
  function g() { return b === null }
  let r = g()
  return r
}
if (m(1) !== true) { throw "missing captured param not bound to null" }
"#,
    );
}

/// A block-scoped `let` shadowing the captured param: the shadow is undone at block
/// exit and the param's (copied-in) binding is restored.
#[test]
fn block_shadow_of_captured_param_restores() {
    run_src(
        r#"
function outer(p) {
  function get() { return p }
  let inner = 0
  {
    let p = 100
    inner = p
  }
  let r = get() + inner
  return r
}
if (outer(7) !== 107) { throw "block shadow of captured param broken: " + outer(7) }
"#,
    );
}
