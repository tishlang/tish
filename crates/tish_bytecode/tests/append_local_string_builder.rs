//! #186 — the plain-assign string-builder fast path: `s = s + <str>` compiles to `AppendLocal`, but a
//! numeric accumulator `i = i + 1` must NOT (or the VM's single string-builder slot thrashes → O(n²)).

use tishlang_bytecode::{compile, Chunk, Opcode};
use tishlang_parser::parse;

fn count_opcode(chunk: &Chunk, op: u8) -> usize {
    // Opcodes are variable-width; the compiler here emits only slot-carrying ops, so a byte-value
    // scan is a sufficient upper bound for these targeted single-loop programs. We assert presence
    // vs absence, not exact counts, so an incidental operand collision can't cause a false pass.
    let mut n = chunk.code.iter().filter(|&&b| b == op).count();
    for nested in &chunk.nested {
        n += count_opcode(nested, op);
    }
    n
}

fn chunk_of(src: &str) -> Chunk {
    let program = parse(src).expect("parse");
    let optimized = tishlang_opt::optimize(&program);
    compile(&optimized).expect("compile")
}

#[test]
fn string_plus_assign_uses_append_local() {
    // `s = s + "x"` must route through the builder (the string_concat 55x→2.5x win).
    let chunk = chunk_of("let s = \"\"\nlet i = 0\nwhile (i < 10) { s = s + \"x\"; i = i + 1 }\n");
    assert!(
        count_opcode(&chunk, Opcode::AppendLocal as u8) >= 1,
        "`s = s + \"x\"` must emit AppendLocal"
    );
}

#[test]
fn numeric_plus_assign_does_not_use_append_local() {
    // A pure numeric loop must NOT builder-ize — no AppendLocal at all.
    let chunk = chunk_of("let n = 0\nlet i = 0\nwhile (i < 10) { n = n + 2; i = i + 1 }\n");
    assert_eq!(
        count_opcode(&chunk, Opcode::AppendLocal as u8),
        0,
        "`n = n + 2` / `i = i + 1` must NOT emit AppendLocal (would thrash the single builder slot)"
    );
}

#[test]
fn template_literal_rhs_uses_append_local() {
    let chunk = chunk_of("let t = \"\"\nlet j = 0\nwhile (j < 3) { t = t + `[${j}]`; j = j + 1 }\n");
    assert!(
        count_opcode(&chunk, Opcode::AppendLocal as u8) >= 1,
        "`t = t + `[${{j}}]`` (template RHS) must emit AppendLocal"
    );
}

#[test]
fn prepend_does_not_use_append_local() {
    // `p = "a" + p` is a prepend — the builder appends, so it must NOT match.
    let chunk = chunk_of("let p = \"z\"\np = \"a\" + p\n");
    assert_eq!(
        count_opcode(&chunk, Opcode::AppendLocal as u8),
        0,
        "prepend `p = \"a\" + p` must not use the append builder"
    );
}
