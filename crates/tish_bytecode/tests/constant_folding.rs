//! Verify AST optimization (constant folding) yields expected bytecode.
//! Uses tish_opt::optimize before compile to match the pipeline used by run/compile.

use tish_bytecode::{compile, Chunk, Opcode};
use tish_parser::parse;

fn chunk_contains_opcode(chunk: &Chunk, op: u8) -> bool {
    if chunk.code.contains(&op) {
        return true;
    }
    for nested in &chunk.nested {
        if chunk_contains_opcode(nested, op) {
            return true;
        }
    }
    false
}

/// 1 + 2 should fold to constant 3; no BinOp in bytecode.
#[test]
fn constant_fold_binary_no_binop() {
    let source = "1 + 2";
    let program = parse(source).expect("parse");
    let optimized = tish_opt::optimize(&program);
    let chunk = compile(&optimized).expect("compile");
    assert!(
        !chunk_contains_opcode(&chunk, Opcode::BinOp as u8),
        "Expected no BinOp for 1+2 after constant folding"
    );
    assert!(
        chunk.constants.iter().any(|c| matches!(c, tish_bytecode::Constant::Number(n) if (*n - 3.0).abs() < f64::EPSILON)),
        "Expected constant 3 in chunk"
    );
}

/// Peephole: Dup+Pop should be removed (replaced with Nop Nop).
/// Compile a program that may emit Dup+Pop; verify it runs and chunk has no consecutive Dup,Pop.
fn chunk_has_dup_pop_sequence(code: &[u8]) -> bool {
    let dup = Opcode::Dup as u8;
    let pop = Opcode::Pop as u8;
    for i in 0..code.len().saturating_sub(1) {
        if code[i] == dup && code[i + 1] == pop {
            return true;
        }
    }
    false
}

fn chunk_contains_dup_pop(chunk: &Chunk) -> bool {
    if chunk_has_dup_pop_sequence(&chunk.code) {
        return true;
    }
    for nested in &chunk.nested {
        if chunk_contains_dup_pop(nested) {
            return true;
        }
    }
    false
}

#[test]
fn peephole_remove_dup_pop() {
    let source = "let o = {a:1}; o?.a";
    let program = parse(source).expect("parse");
    let optimized = tish_opt::optimize(&program);
    let chunk = compile(&optimized).expect("compile");
    assert!(
        !chunk_contains_dup_pop(&chunk),
        "Peephole should remove Dup+Pop sequences"
    );
}

/// -42 should fold to constant -42; no UnaryOp in bytecode.
#[test]
fn constant_fold_unary_no_unaryop() {
    let source = "-42";
    let program = parse(source).expect("parse");
    let optimized = tish_opt::optimize(&program);
    let chunk = compile(&optimized).expect("compile");
    assert!(
        !chunk_contains_opcode(&chunk, Opcode::UnaryOp as u8),
        "Expected no UnaryOp for -42 after constant folding"
    );
}
