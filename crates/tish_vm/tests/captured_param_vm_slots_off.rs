//! #716 follow-up parity guard: the captured-param mutation-semantics shapes must behave
//! identically with `TISH_VM_SLOTS=0` (the legacy name-based path). This file holds
//! exactly ONE test so the process-global env var cannot race a sibling test — cargo
//! builds each file in `tests/` as its own binary/process.
//!
//! (The missing-arg shape is deliberately absent: the legacy name-based path leaves a
//! missing param unbound — "Undefined variable" — a pre-existing divergence from the
//! interpreter that slot mode does not share.)

use tishlang_bytecode::compile;

fn run_src(src: &str) {
    let program = tishlang_parser::parse(src).expect("parse");
    let chunk = compile(&program).expect("compile");
    tishlang_vm::run(&chunk).expect("run");
}

#[test]
fn captured_param_semantics_identical_with_slots_off() {
    std::env::set_var("TISH_VM_SLOTS", "0");
    run_src(
        r#"
function outer(p) {
  function get() { return p }
  p = p + 1
  let r = get()
  return r
}
if (outer(1) !== 2) { throw "slots-off: closure saw stale param: " + outer(1) }

function outer2(p) {
  function bump() { p = p + 10 }
  bump()
  let r = p
  return r
}
if (outer2(1) !== 11) { throw "slots-off: body saw stale param: " + outer2(1) }

function mk(p) {
  function get() { return p }
  p = p * 2
  return get
}
let g = mk(4)
if (g() !== 8) { throw "slots-off: escaping closure lost reassigned param: " + g() }

function withDefault(a, p = 5) {
  function get() { return p }
  let r = a + get()
  return r
}
if (withDefault(1) !== 6) { throw "slots-off: captured-param default lost" }
if (withDefault(1, 2) !== 3) { throw "slots-off: captured-param supplied arg broken" }

function recur(n) {
  function dec() { return n - 1 }
  if (n === 0) { return 0 }
  let x = recur(dec())
  return x + n
}
if (recur(3) !== 6) { throw "slots-off: recursion with captured param broken" }
"#,
    );
}
