//! #716: a body-level `function NAME` used to be bound with `DeclareVar` into the very
//! `local_scope` map its own closure had just captured (`Constant::Closure` →
//! `VmClosure.enclosing`), minting a deterministic closure→scope→closure reference cycle
//! per call. The runtime is refcount-only, so every such call frame — including everything
//! reachable from its name-based locals — leaked (measured ~6.7 KB/call in the issue).
//! In general slot mode an UNCAPTURED function name is now slot-bound instead, so the
//! closure never enters the captured map and the frame drops normally on return.

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

/// The leak regression itself: N calls of a function with an uncaptured body-level named
/// fn must not retain the N frames. The probe array is reachable from each frame's
/// captured local `s`; before the fix `helper`'s `DeclareVar` completed the cycle and
/// `VmRef::strong_count(&probe)` ended at `2 + N` — every frame stayed alive.
#[test]
fn uncaptured_body_fn_frames_are_reclaimed() {
    if slots_disabled() {
        return;
    }
    let probe: VmRef<Vec<Value>> = VmRef::new(vec![Value::Number(1.0)]);
    let mut vm = Vm::new();
    vm.set_global(Arc::from("probe"), Value::Array(probe.clone()));
    let src = r#"
function work() {
  let s = probe
  function helper() { return s.length }
  return helper()
}
let i = 0
let acc = 0
while (i < 100) { acc = acc + work(); i = i + 1 }
if (acc !== 100) { throw "bad acc: " + acc }
"#;
    let program = tishlang_parser::parse(src).expect("parse");
    let chunk = compile(&program).expect("compile");
    vm.run(&chunk).expect("run");
    // Exactly two live references: this test's `probe` clone + the VM globals entry.
    // Each leaked frame would hold one more (its `local_scope` binds `s` → probe).
    assert_eq!(
        VmRef::strong_count(&probe),
        2,
        "call frames retained the probe — the #716 frame-scope↔closure cycle is back"
    );
}

/// Self-recursion keeps the scope path (its own name IS a capture) and still works.
#[test]
fn self_recursive_body_fn_still_works() {
    run_src(
        r#"
function outer(n) {
  function fact(k) { if (k < 2) { return 1 } return k * fact(k - 1) }
  return fact(n)
}
if (outer(6) !== 720) { throw "self-recursion broken" }
"#,
    );
}

/// Mutual recursion: both names are captured by the sibling closure; both keep the scope path.
#[test]
fn mutually_recursive_body_fns_still_work() {
    run_src(
        r#"
function outer(n) {
  function isEven(k) { if (k === 0) { return true } return isOdd(k - 1) }
  function isOdd(k) { if (k === 0) { return false } return isEven(k - 1) }
  return isEven(n)
}
if (outer(10) !== true) { throw "mutual recursion broken (even)" }
if (outer(7) !== false) { throw "mutual recursion broken (odd)" }
"#,
    );
}

/// tish binds body-level fn decls sequentially (no JS hoisting): a use BEFORE the
/// declaration statement sees the outer binding, exactly as with `let`. The slot-bound
/// variant must preserve that — the reference site compiles before the slot exists, so
/// it stays a name-based load and resolves outward.
#[test]
fn body_fn_binding_is_sequential_not_hoisted() {
    run_src(
        r#"
let f = "global"
function outer() {
  let before = f
  function f() { return "local" }
  return before + "|" + f()
}
if (outer() !== "global|local") { throw "sequential fn binding broken: " + outer() }
"#,
    );
}

/// An uncaptured body-level fn is callable, reassignable, passable as a value, and a
/// block-scoped one stays confined to its block — identical to the name-based path.
#[test]
fn body_fn_value_semantics_unchanged() {
    run_src(
        r#"
function outer() {
  function f() { return 1 }
  let a = f()
  f = () => 2
  let b = f()
  function double(v) { return v * 2 }
  let out = [1, 2, 3].map(double)
  let got = 0
  {
    function inner() { return 39 }
    got = inner()
  }
  return a + b + got + out[0] + out[1] + out[2]
}
if (outer() !== 54) { throw "body fn value semantics broken: " + outer() }
"#,
    );
}

/// A slot-bound fn that ESCAPES its frame (returned) still resolves its captures: the
/// closure carries the captured `local_scope` chain; only the name binding moved to a slot.
#[test]
fn escaping_body_fn_keeps_captures() {
    run_src(
        r#"
function makeGetter() {
  let n = 7
  function get() { return n }
  return get
}
let g = makeGetter()
if (g() !== 7) { throw "escaping body fn lost its capture" }
"#,
    );
}
