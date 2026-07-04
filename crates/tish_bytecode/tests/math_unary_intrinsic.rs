//! #186 — `Math.<unaryfn>(arg)` lowers to `MathUnary` ONLY when `Math` is the unshadowed global; a
//! rebound `Math`, a 2-arg `Math.*`, or a non-math member must keep the general call path.

use tishlang_bytecode::{compile, Chunk, Opcode};
use tishlang_parser::parse;

fn has_math_unary(chunk: &Chunk) -> bool {
    if chunk.code.contains(&(Opcode::MathUnary as u8)) {
        return true;
    }
    chunk.nested.iter().any(has_math_unary)
}

fn chunk_of(src: &str) -> Chunk {
    let program = parse(src).expect("parse");
    let optimized = tishlang_opt::optimize(&program);
    compile(&optimized).expect("compile")
}

#[test]
fn global_math_unary_emits_intrinsic() {
    assert!(has_math_unary(&chunk_of("let x = Math.sqrt(4.0)\n")), "Math.sqrt → MathUnary");
    assert!(has_math_unary(&chunk_of("let x = Math.sin(1.0) + Math.floor(2.5)\n")));
}

#[test]
fn shadowed_math_keeps_general_call() {
    // `Math` rebound as a local → the intrinsic would be a miscompile; must NOT emit MathUnary.
    assert!(
        !has_math_unary(&chunk_of("let Math = 5\nlet x = Math\n")),
        "a program that rebinds `Math` must not intrinsify"
    );
    assert!(
        !has_math_unary(&chunk_of(
            "fn mysqrt(x) { return x }\nlet Math = { sqrt: mysqrt }\nlet x = Math.sqrt(4.0)\n"
        )),
        "a shadowed Math.sqrt must call the user's method"
    );
}

#[test]
fn non_unary_math_stays_a_call() {
    // Math.max is 2-arg (not a unary intrinsic) → general call, no MathUnary.
    assert!(!has_math_unary(&chunk_of("let x = Math.max(1.0, 2.0)\n")), "Math.max is not unary");
    // Math.PI is a property, not a call.
    assert!(!has_math_unary(&chunk_of("let x = Math.PI\n")), "Math.PI is a property");
}
