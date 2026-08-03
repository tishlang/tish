//! Regression: `&&` / `||` yield the deciding OPERAND (JS semantics, #240). #328 fixed the left
//! operand to bind by `.clone()` rather than by move; the right operand had the identical hazard.
//!
//! When the right operand lowers to a bare local — `inGenBlock && currentTrack` — yielding it moved
//! that local out of scope, so any later read was `E0382: use of moved value`. It only bites when
//! the operand is a plain non-`Copy` `Value` local that is read again (typically the next iteration
//! of the enclosing loop), which is why it survived: the common right operand is a call or an
//! already-cloned expression, which are temporaries.
//!
//! Found compiling `@spacedevin/deck`'s `Parser.tish`, whose statement loop does exactly this. The
//! source runs fine under `--target js` and the interpreter, so the divergence was Rust-backend-only.
use tishlang_compile::compile;
use tishlang_parser::parse;

fn rust_for(src: &str) -> String {
    compile(&parse(src).unwrap()).unwrap()
}

#[test]
fn bare_ident_right_operand_is_cloned_not_moved() {
    let rust = rust_for(
        r#"
fn f() {
  let cur = null
  let flag = true
  let i = 0
  while (i < 3) {
    if (flag && cur) { return 1 }
    if (flag && cur) { return 2 }
    i = i + 1
  }
  return 0
}
"#,
    );
    assert!(
        rust.contains("if __l.is_truthy() { cur.clone() }"),
        "a bare-ident right operand must be cloned out, not moved:\n{rust}"
    );
    assert!(
        !rust.contains("if __l.is_truthy() { cur }"),
        "found the moving form, which is E0382 on the second read:\n{rust}"
    );
}

#[test]
fn or_right_operand_gets_the_same_treatment() {
    let rust = rust_for(
        r#"
fn f() {
  let a = null
  let b = null
  let i = 0
  while (i < 3) {
    let x = a || b
    let y = a || b
    i = i + 1
  }
  return 0
}
"#,
    );
    assert!(
        rust.contains("else { b.clone() }"),
        "`||` must clone a bare-ident right operand too:\n{rust}"
    );
}

#[test]
fn non_ident_right_operands_are_not_given_a_redundant_clone() {
    // A call result is already a temporary — cloning it would be pure overhead on a hot path.
    let rust = rust_for(
        r#"
fn g() { return 1 }
fn f() {
  let flag = true
  return flag && g()
}
"#,
    );
    assert!(
        !rust.contains("))).clone().clone()"),
        "an already-temporary right operand must not gain a second clone:\n{rust}"
    );
}
