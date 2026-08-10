//! #641 — call arity. A call with too few arguments used to compile, bind the missing parameter to
//! `Value::Null`, and then either panic at runtime (boxed callee, typed params), fail in rustc
//! (direct native callee), or silently yield null/NaN (interpreter) — depending on which lowering
//! the CALLEE happened to get. These pin the rule and, just as importantly, the non-rule: omitting
//! an argument bound to an UNTYPED parameter stays legal, because shipping packages rely on it.

use tishlang_compile::check_call_arity;
use tishlang_parser::parse;

fn diags(src: &str) -> Vec<String> {
    let p = parse(src).expect("parse");
    check_call_arity(&p).into_iter().map(|d| d.message).collect()
}

#[test]
fn omitting_a_typed_parameter_is_an_error() {
    let d = diags(
        r#"
function paintTyped(slot: i32, rv: i32): i32 { return slot + rv }
paintTyped(1)
"#,
    );
    assert_eq!(d.len(), 1, "expected exactly one diagnostic, got {:?}", d);
    assert!(d[0].contains("paintTyped"), "names the callee: {}", d[0]);
    assert!(d[0].contains("needs 2"), "states the requirement: {}", d[0]);
}

#[test]
fn omitting_an_untyped_parameter_is_allowed() {
    // The established idiom: `packages/shop.tish` has six of these, `packages/ui.tish` one.
    // Enforcing arity here would break shipping code that is behaving exactly as intended.
    let d = diags(
        r#"
function renderTab(defer, stream) { return 1 }
renderTab()
renderTab(1)
renderTab(1, 1)
"#,
    );
    assert!(d.is_empty(), "untyped params must stay optional, got {:?}", d);
}

#[test]
fn a_defaulted_parameter_imposes_no_minimum() {
    // The default is what fills the slot, so nothing arrives as null.
    let d = diags(
        r#"
function f(a: i32, b: i32 = 7): i32 { return a + b }
f(1)
"#,
    );
    assert!(d.is_empty(), "a default supplies the argument, got {:?}", d);
}

#[test]
fn a_typed_parameter_after_untyped_ones_still_forces_its_position() {
    // A call cannot skip positions to reach `c`, so the minimum is 3 even though 0 and 1 are
    // untyped — the minimum is one past the LAST typed parameter, not a count of typed ones.
    let d = diags(
        r#"
function g(a, b, c: i32): i32 { return c }
g(1, 2)
"#,
    );
    assert_eq!(d.len(), 1, "expected one diagnostic, got {:?}", d);
    assert!(d[0].contains("needs 3"), "minimum is positional: {}", d[0]);
}

#[test]
fn a_spread_argument_is_not_second_guessed() {
    // The count is unknowable at compile time; saying nothing beats guessing wrong.
    let d = diags(
        r#"
function h(a: i32, b: i32): i32 { return a + b }
let xs = [1, 2]
h(...xs)
"#,
    );
    assert!(d.is_empty(), "spread must not be flagged, got {:?}", d);
}

#[test]
fn a_nested_call_is_still_seen() {
    // The reported shape. `for_each_stmt_expr` hands out only a statement's ROOT expression, so
    // without full sub-expression walking this exact case — the one in the issue — is missed.
    let d = diags(
        r#"
function paintTyped(slot: i32, rv: i32): i32 { return slot + rv }
console.log("v = " + paintTyped(1))
"#,
    );
    assert_eq!(d.len(), 1, "nested call must be checked, got {:?}", d);
}

#[test]
fn a_correct_call_is_silent() {
    let d = diags(
        r#"
function paintTyped(slot: i32, rv: i32): i32 { return slot + rv }
paintTyped(1, 2)
"#,
    );
    assert!(d.is_empty(), "correct arity must not warn, got {:?}", d);
}
