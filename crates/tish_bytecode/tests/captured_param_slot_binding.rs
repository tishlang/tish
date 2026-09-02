//! #716 follow-up: a function whose PARAM is captured by a nested closure must still
//! compile in general slot mode, with a captured-param COPY-IN PROLOGUE (`LoadLocal
//! <param slot>` + `DeclareVarPlain <name>`) so the closure reads the param from
//! `local_scope` while the chunk keeps frame slots for everything uncaptured —
//! including the body-level helper NAME (#728), which is what breaks the per-call
//! frame-scope↔closure cycle for the #716 repro shape.

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

/// The #716 repro shape: the chunk must stay slot-based, copy the captured param in
/// exactly once, and slot-bind the (uncaptured) helper name — zero `DeclareVar`.
#[test]
fn captured_param_chunk_stays_slot_based_with_copy_in() {
    if slots_disabled() {
        return;
    }
    let chunk = compile_src(
        r#"
function handleEvent(payload) {
  function fmt(x) { return x + payload.tag }
  return fmt("v")
}
console.log(handleEvent({ tag: "t" }))
"#,
    );
    let outer = &chunk.nested[0];
    assert!(
        outer.slot_based,
        "a captured-param chunk must no longer be disqualified from general slot mode"
    );
    assert_eq!(
        count_opcode(outer, Opcode::DeclareVarPlain),
        1,
        "exactly one copy-in store for the one captured param"
    );
    assert_eq!(
        count_opcode(outer, Opcode::DeclareVar),
        0,
        "the uncaptured helper name must be slot-bound (#728), not DeclareVar-bound — \
         that is the cycle"
    );
    assert!(
        count_opcode(outer, Opcode::StoreLocal) >= 1,
        "the helper closure should be stored to a frame slot"
    );
}

/// Mixed params: only the CAPTURED one is copied in; the uncaptured one stays
/// slot-resolved (no name-based traffic for it).
#[test]
fn only_captured_params_are_copied_in() {
    if slots_disabled() {
        return;
    }
    let chunk = compile_src(
        r#"
function outer(p, q) {
  function get() { return p }
  let r = get() + q
  return r
}
console.log(outer(1, 2))
"#,
    );
    let outer = &chunk.nested[0];
    assert!(outer.slot_based, "mixed-param chunk should be slot-based");
    assert_eq!(
        count_opcode(outer, Opcode::DeclareVarPlain),
        1,
        "only the captured param `p` gets a copy-in store; `q` stays slotted"
    );
}

/// A self-recursive helper capturing the param: the helper NAME is captured (its own
/// body references it) and stays on the `DeclareVar` scope path, while the chunk itself
/// remains slot-based with the param copy-in.
#[test]
fn self_recursive_helper_keeps_scope_path_beside_copy_in() {
    if slots_disabled() {
        return;
    }
    let chunk = compile_src(
        r#"
function outer(n) {
  function fact(k) { if (k < 2) { return n } return k * fact(k - 1) }
  return fact(4)
}
console.log(outer(1))
"#,
    );
    let outer = &chunk.nested[0];
    assert!(outer.slot_based, "chunk should stay slot-based");
    assert_eq!(
        count_opcode(outer, Opcode::DeclareVarPlain),
        1,
        "the captured param `n` gets its copy-in"
    );
    assert_eq!(
        count_opcode(outer, Opcode::DeclareVar),
        1,
        "the self-recursive helper name is captured and stays DeclareVar-bound"
    );
}
