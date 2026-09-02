//! #727: WHERE the default-parameter prologue stores its value in general slot mode.
//!
//! In a general-slot-mode chunk every read of a param resolves to `LoadLocal <slot>`, so
//! the applied default must be stored with `StoreLocal <slot>` — a `DeclareVarPlain`
//! writes it into the name-based scope map where no read will ever find it (the slot
//! keeps its missing-arg `Null`; `a + b` → NaN).

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
fn general_slot_mode_default_stores_to_slot() {
    if slots_disabled() {
        return;
    }
    let chunk = compile_src(
        r#"
function f(a, b = 5) {
  let c = a + b
  return c
}
console.log(f(1))
"#,
    );
    let f = &chunk.nested[0];
    assert!(f.slot_based, "f should compile in general slot mode");
    assert_eq!(
        count_opcode(f, Opcode::DeclareVarPlain),
        0,
        "the applied default must not be declared into the name-based scope map — \
         reads of the param resolve to its frame slot (#727)"
    );
    assert!(
        count_opcode(f, Opcode::StoreLocal) >= 1,
        "the applied default should be stored to the param's frame slot"
    );
}

/// The slot-DISQUALIFIED shape (captured param → name-based chunk) must keep the
/// `DeclareVarPlain` store: there is no slot, and closures read the param by name.
#[test]
fn name_based_chunk_default_still_declares_by_name() {
    let chunk = compile_src(
        r#"
function g(a, b = 5) {
  function h() { return b }
  let c = a + h()
  return c
}
console.log(g(1))
"#,
    );
    let g = &chunk.nested[0];
    assert_eq!(
        count_opcode(g, Opcode::DeclareVarPlain),
        1,
        "captured-param chunk is name-based; its default must stay DeclareVarPlain-bound"
    );
}
