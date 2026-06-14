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

// Regression: `continue` inside a `do { } while (cond)` targets the condition test, which is emitted
// AFTER the body — a FORWARD jump. It used to be lowered as a backward `JumpBack`, which `patch_jump_back`
// resolved to distance 0 (saturating_sub of a forward target) — a no-op. Execution then fell through into
// the body block's already-unwound `ExitBlock`, crashing the VM with "ExitBlock without matching
// EnterBlock". The `continue` here unwinds the body block AND the `if` then-block, so it exercises the
// multi-ExitBlock unwind path specifically.
#[test]
fn do_while_continue_does_not_crash_vm() {
    let src = "let d = 0\n\
               do {\n\
                 d = d + 1\n\
                 if (d === 2) { continue }\n\
               } while (d < 5)\n";
    let prog = tishlang_parser::parse(src).expect("parse");
    // Both compile paths must run clean (the crash happened regardless of peephole optimization).
    run(&compile_unoptimized(&prog).expect("compile (unopt)")).expect("VM run unoptimized");
    run(&compile(&prog).expect("compile (opt)")).expect("VM run optimized");
}
