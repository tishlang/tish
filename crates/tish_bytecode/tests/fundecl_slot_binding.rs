//! #716: how a body-level `function NAME` binds its name in general slot mode.
//!
//! An UNCAPTURED name must be slot-bound (`StoreLocal`) — `DeclareVar` would insert the
//! closure into the very `local_scope` map the closure captured, minting a per-call
//! closure→scope→closure cycle that leaks the whole frame (refcount-only runtime, no
//! cycle collector). A name referenced from inside ANY nested closure — the function's
//! own body (self-recursion) or a sibling (mutual recursion) — is in the captured set
//! and must STAY on the `DeclareVar` scope path, or the recursive name lookup breaks.

use tishlang_bytecode::{compile, Chunk, Opcode};

/// Count occurrences of `target` in `chunk.code`, walking real instruction boundaries.
fn count_opcode(chunk: &Chunk, target: Opcode) -> usize {
    let code = &chunk.code;
    let mut ip = 0usize;
    let mut n = 0usize;
    while ip < code.len() {
        let op = Opcode::from_u8(code[ip]).expect("valid opcode");
        if op == target {
            n += 1;
        }
        ip += op.instruction_size(code, ip).expect("sized instruction");
    }
    n
}

fn compile_src(src: &str) -> Chunk {
    let program = tishlang_parser::parse(src).expect("parse");
    compile(&program).expect("compile")
}

/// These tests assert slot-mode emission; running the suite with `TISH_VM_SLOTS=0`
/// deliberately exercises the legacy name-based path, where they do not apply.
fn slots_disabled() -> bool {
    std::env::var("TISH_VM_SLOTS")
        .map(|v| v == "0")
        .unwrap_or(false)
}

#[test]
fn uncaptured_body_level_fn_is_slot_bound() {
    if slots_disabled() {
        return;
    }
    let chunk = compile_src(
        r#"
function outer() {
  function helper(a) { return a + 1 }
  return helper(41)
}
console.log(outer())
"#,
    );
    let outer = &chunk.nested[0];
    assert!(
        outer.slot_based,
        "outer should compile in general slot mode"
    );
    assert_eq!(
        count_opcode(outer, Opcode::DeclareVar),
        0,
        "uncaptured `function helper` must not DeclareVar into the captured scope (#716 cycle)"
    );
    assert!(
        count_opcode(outer, Opcode::StoreLocal) >= 1,
        "uncaptured `function helper` should be stored to a frame slot"
    );
}

#[test]
fn self_recursive_body_level_fn_stays_on_scope_path() {
    if slots_disabled() {
        return;
    }
    let chunk = compile_src(
        r#"
function outer() {
  function fact(k) { if (k < 2) { return 1 } return k * fact(k - 1) }
  return fact(5)
}
console.log(outer())
"#,
    );
    let outer = &chunk.nested[0];
    assert_eq!(
        count_opcode(outer, Opcode::DeclareVar),
        1,
        "self-recursive `fact` references its own name from inside its body — that IS a \
         capture, and the name must stay DeclareVar-bound in `local_scope`"
    );
}

#[test]
fn mutually_recursive_body_level_fns_stay_on_scope_path() {
    if slots_disabled() {
        return;
    }
    let chunk = compile_src(
        r#"
function outer(n) {
  function isEven(k) { if (k === 0) { return true } return isOdd(k - 1) }
  function isOdd(k) { if (k === 0) { return false } return isEven(k - 1) }
  return isEven(n)
}
console.log(outer(10))
"#,
    );
    let outer = &chunk.nested[0];
    assert_eq!(
        count_opcode(outer, Opcode::DeclareVar),
        2,
        "mutually recursive names are captured by the sibling closure and must both stay \
         DeclareVar-bound"
    );
}
