//! Regression: C-style `for` `continue` must jump forward to the update clause (not JumpBack).

use std::fs;

use tishlang_bytecode::{compile, compile_unoptimized};
use tishlang_vm::run;

#[test]
fn break_continue_fixture_runs_on_vm() {
    let path = format!(
        "{}/../../tests/core/break_continue.tish",
        env!("CARGO_MANIFEST_DIR")
    );
    let src = fs::read_to_string(&path).unwrap();
    let prog = tishlang_parser::parse(&src).expect("parse");
    let chunk = compile_unoptimized(&prog).expect("compile");
    run(&chunk).expect("VM run");
}

#[test]
fn mutation_vm_ast_opt_without_peephole() {
    let path = format!(
        "{}/../../tests/core/mutation.tish",
        env!("CARGO_MANIFEST_DIR")
    );
    let src = fs::read_to_string(&path).unwrap();
    let mut prog = tishlang_parser::parse(&src).expect("parse");
    tishlang_opt::optimize(&mut prog);
    let chunk = compile_unoptimized(&prog).expect("compile");
    run(&chunk).expect("VM");
}

#[test]
fn mutation_vm_ast_opt_with_peephole() {
    let path = format!(
        "{}/../../tests/core/mutation.tish",
        env!("CARGO_MANIFEST_DIR")
    );
    let src = fs::read_to_string(&path).unwrap();
    let mut prog = tishlang_parser::parse(&src).expect("parse");
    tishlang_opt::optimize(&mut prog);
    let chunk = compile(&prog).expect("compile");
    run(&chunk).expect("VM");
}
