//! Verify arr.sort((a,b)=>a-b) compiles to ArraySortNumeric (opcode 31).

use tish_bytecode::{compile, Opcode};
use tish_parser::parse;

fn chunk_contains_opcode(chunk: &tish_bytecode::Chunk, op: u8) -> bool {
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

#[test]
fn test_numeric_sort_uses_array_sort_numeric() {
    let source = r#"
        let x = [3, 1, 2];
        x.sort((a, b) => a - b);
        console.log(x);
    "#;
    let program = parse(source).expect("parse");
    let chunk = compile(&program).expect("compile");
    assert!(
        chunk_contains_opcode(&chunk, Opcode::ArraySortNumeric as u8),
        "Expected ArraySortNumeric (31) in bytecode for x.sort((a,b)=>a-b)"
    );
}
