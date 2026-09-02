//! #727 parity guard: the same default-parameter shapes must behave identically with
//! `TISH_VM_SLOTS=0` (the legacy name-based path). This file holds exactly ONE test so
//! the process-global env var cannot race a sibling test — cargo builds each file in
//! `tests/` as its own binary/process.

use tishlang_bytecode::compile;

fn run_src(src: &str) {
    let program = tishlang_parser::parse(src).expect("parse");
    let chunk = compile(&program).expect("compile");
    tishlang_vm::run(&chunk).expect("run");
}

#[test]
fn param_defaults_behave_identically_with_slots_off() {
    std::env::set_var("TISH_VM_SLOTS", "0");
    run_src(
        r#"
function f(a, b = 5) {
  let c = a + b
  return c
}
if (f(1) !== 6) { throw "slots-off default broken: f(1) = " + f(1) }
if (f(1, 2) !== 3) { throw "slots-off supplied arg broken: f(1, 2) = " + f(1, 2) }

function dep(a, b = a + 1) {
  let c = a + b
  return c
}
if (dep(3) !== 7) { throw "slots-off dependent default broken: dep(3) = " + dep(3) }
if (dep(3, 10) !== 13) { throw "slots-off dependent supplied broken" }

function g(a, b = 5) {
  function h() { return b }
  let c = a + h()
  return c
}
if (g(1) !== 6) { throw "slots-off captured-param default broken: g(1) = " + g(1) }

function po(a, b = 5) { return a + b }
if (po(1) !== 6) { throw "slots-off param-only default broken" }
"#,
    );
}
