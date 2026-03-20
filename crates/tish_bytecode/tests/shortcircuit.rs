//! Verify && and || short-circuit (JumpIfFalse before evaluating right side).
use std::path::Path;
use tish_bytecode::{compile, compile_unoptimized, Opcode};
use tish_compile::{merge_modules, resolve_project};
use tish_parser::parse;
use tish_opt;
use tish_vm;

#[test]
fn test_and_shortcircuit_emits_jump() {
    let source = "let x = null; let y = x != null && x.foo;";
    let program = parse(source).expect("parse");
    let chunk = compile_unoptimized(&program).expect("compile");
    let code = &chunk.code;
    let has_jump_if_false = code.windows(1).any(|w| w[0] == Opcode::JumpIfFalse as u8);
    assert!(has_jump_if_false, "And should emit JumpIfFalse for short-circuit");
}

#[test]
fn test_and_shortcircuit_runs_unoptimized() {
    let source = "let x = null; let y = x != null && x.foo;";
    let program = parse(source).expect("parse");
    let chunk = compile_unoptimized(&program).expect("compile");
    let result = tish_vm::run(&chunk);
    assert!(result.is_ok(), "Should not throw (short-circuit avoids x.foo): {:?}", result.err());
}

#[test]
fn test_and_shortcircuit_runs_optimized() {
    let source = "let x = null; let y = x != null && x.foo;";
    let program = parse(source).expect("parse");
    let program = tish_opt::optimize(&program);
    let chunk = tish_bytecode::compile(&program).expect("compile");
    let result = tish_vm::run(&chunk);
    assert!(result.is_ok(), "Should not throw with peephole (short-circuit): {:?}", result.err());
}

#[test]
fn test_and_shortcircuit_via_resolve_project() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/shortcircuit.tish");
    let path = path.canonicalize().expect("path");
    let project_root = path.parent().unwrap();
    let modules = resolve_project(&path, Some(project_root)).expect("resolve");
    let program = merge_modules(modules).expect("merge");
    let program = tish_opt::optimize(&program); // Mirror CLI
    let chunk = compile(&program).expect("compile");
    let result = tish_vm::run(&chunk);
    assert!(result.is_ok(), "Should not throw via resolve+merge+opt (CLI path): {:?}", result.err());
}
