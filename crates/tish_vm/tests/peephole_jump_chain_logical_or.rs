//! Regression: bytecode peephole `chain_jumps` must not follow `JumpIfFalse` as if it were an
//! unconditional `Jump`. Doing so broke `===` + `||` (VM differed from interpreter).

use tishlang_bytecode::{
    compile, compile_for_repl, compile_for_repl_unoptimized, compile_unoptimized,
};
use tishlang_core::Value;

fn run_chunk(chunk: &tishlang_bytecode::Chunk) -> Value {
    tishlang_vm::run(chunk).expect("vm run")
}

#[test]
fn logical_or_strict_eq_peephole_matches_unoptimized() {
    let src = "let a = 1\nlet b = 2\na === 1 || b === 2";
    let program = tishlang_parser::parse(src).expect("parse");
    let program = tishlang_opt::optimize(&program);

    let v_peep = run_chunk(&compile(&program).expect("compile"));
    let v_raw = run_chunk(&compile_unoptimized(&program).expect("compile unopt"));
    assert!(
        v_peep.strict_eq(&v_raw),
        "peephole changed semantics: peep={v_peep:?} raw={v_raw:?}"
    );

    let v_peep_repl = run_chunk(&compile_for_repl(&program).expect("compile repl"));
    let v_raw_repl = run_chunk(&compile_for_repl_unoptimized(&program).expect("compile repl unopt"));
    assert!(
        v_peep_repl.strict_eq(&v_raw_repl),
        "repl: peep={v_peep_repl:?} raw={v_raw_repl:?}"
    );
}

#[test]
fn logical_or_inside_if_condition_peephole_matches_unoptimized() {
    let src = "let a = 1\nlet b = 2\nif (a === 1 || b === 2) { 1 } else { 0 }";
    let program = tishlang_parser::parse(src).expect("parse");
    let program = tishlang_opt::optimize(&program);

    let v_peep = run_chunk(&compile(&program).expect("compile"));
    let v_raw = run_chunk(&compile_unoptimized(&program).expect("compile unopt"));
    assert!(
        v_peep.strict_eq(&v_raw),
        "if + || : peep={v_peep:?} raw={v_raw:?}"
    );
}
