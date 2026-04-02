//! Regression: bytecode peephole `chain_jumps` must not follow `JumpIfFalse` as if it were an
//! unconditional `Jump`. Doing so broke `===` + `||` when nested as the condition of an outer `if`
//! (default VM differed from `--backend interp` / `--no-optimize`).
//!
//! CLI parity for the same source is covered in `crates/tish/tests/run_optimize_stdout_parity.rs`.

use std::path::PathBuf;

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

#[test]
fn string_strict_eq_logical_or_ast_opt_matches_unoptimized_bytecode() {
    let src = "let cmd = \"a\"\nif (cmd === \"a\" || cmd === \"b\") { 1 } else { 0 }";
    let raw = tishlang_parser::parse(src).expect("parse");
    let opt = tishlang_opt::optimize(&raw);
    let v_raw = run_chunk(&compile_unoptimized(&raw).expect("raw"));
    let v_opt = run_chunk(&compile_unoptimized(&opt).expect("opt"));
    assert!(
        v_raw.strict_eq(&v_opt),
        "AST optimizer changed semantics: raw={v_raw:?} opt={v_opt:?}"
    );
}

#[test]
fn string_strict_eq_logical_or_peephole_matches_unoptimized() {
    let src = "let cmd = \"a\"\nif (cmd === \"a\" || cmd === \"b\") { 1 } else { 0 }";
    let program = tishlang_opt::optimize(&tishlang_parser::parse(src).expect("parse"));
    let v_peep = run_chunk(&compile(&program).expect("compile"));
    let v_raw = run_chunk(&compile_unoptimized(&program).expect("unopt"));
    assert!(
        v_peep.strict_eq(&v_raw),
        "peephole + strings: peep={v_peep:?} raw={v_raw:?}"
    );
}

/// `tish run path/to/file.tish` uses merge_modules; ensure that matches plain parse for the fixture.
#[test]
fn merged_module_program_bytecode_matches_parse_for_string_or_fixture() {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/or_string_cmd.tish");
    let src = std::fs::read_to_string(&fixture).expect("read fixture");
    let modules = tishlang_compile::resolve_project(&fixture, Some(fixture.parent().unwrap()))
        .expect("resolve");
    let merged = tishlang_compile::merge_modules(modules).expect("merge");
    let flat = tishlang_parser::parse(&src).expect("parse");
    let m_opt = tishlang_opt::optimize(&merged);
    let f_opt = tishlang_opt::optimize(&flat);
    let c_m = compile(&m_opt).expect("compile merged");
    let c_f = compile(&f_opt).expect("compile flat");
    assert_eq!(
        c_m.code, c_f.code,
        "merge_modules vs parse produced different bytecode"
    );
}
