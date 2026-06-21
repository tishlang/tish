//! #178 — recursive-struct native lowering (behind `TISH_REC_STRUCT`).
//!
//! A developer-shaped recursive binary tree (arbitrary identifiers, NOT the fixture names
//! bottomUpTree/itemCheck/binaryTrees) must lower to a native arena: an `i32`-indexed struct,
//! native `build`/`check` free fns threading `&mut Vec<Node>` / `&Vec<Node>`, and a top-level
//! routed call — with NO `object_from_pairs` / `get_prop` / `value_call` on the hot path. This is
//! the real fix that makes the boxed `binary_trees` path fast under any names (retiring the
//! `binary_trees_check` fixture kernel). See docs/recursive-struct-native.md.

use std::path::PathBuf;

use tishlang_compile::compile_project_full;

fn enable_flags() {
    for k in [
        "TISH_PARAM_NATIVE",
        "TISH_PARAM_INFER",
        "TISH_NATIVE_FN",
        "TISH_STRUCT_INFER",
        "TISH_FUSED_HOF",
        "TISH_NATIVE_HOF",
        "TISH_AGGREGATE_INFER",
        "TISH_REC_STRUCT",
    ] {
        std::env::set_var(k, "1");
    }
}

const SRC: &str = r#"
function buildNode(d) {
  if (d > 0) { return { left: buildNode(d - 1), right: buildNode(d - 1) } }
  return { left: null, right: null }
}
function sumNode(node) {
  if (node.left === null) { return 1 }
  return 1 + sumNode(node.left) + sumNode(node.right)
}
let t0 = Date.now()
let check = sumNode(buildNode(10))
console.log("GAUNTLET rec " + (Date.now() - t0) + " " + check)
"#;

#[test]
fn recursive_tree_lowers_to_native_arena() {
    enable_flags();
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    let path = dir.join("rec_tree_dev_178.tish");
    std::fs::write(&path, SRC).unwrap();

    let (rust, _, _, _) = compile_project_full(&path, path.parent(), &[], true).unwrap();

    // The synthesized arena node struct (i32 child indices, not Option<Box> / not boxed Value).
    assert!(
        rust.contains("struct TishStruct_TishRecNode"),
        "expected a synthesized recursive node struct"
    );
    // Native builder + consumer free fns over the arena.
    assert!(
        rust.contains("fn buildNode_rec(") && rust.contains("__rec_arena: &mut Vec<"),
        "expected a native arena builder fn"
    );
    assert!(
        rust.contains("fn sumNode_rec(__rec_idx: i32, __rec_arena: &Vec<"),
        "expected a native arena consumer fn"
    );
    // Top-level call routed through the arena, not the boxed closure.
    assert!(
        rust.contains("let __rec_root = buildNode_rec("),
        "expected the top-level builder call to be routed to the native arena fn"
    );
    assert!(
        rust.contains("sumNode_rec(__rec_root, &__rec_arena)"),
        "expected the consumer to be invoked on the arena root index"
    );
    // No per-node boxed allocation on the recursive build path.
    assert!(
        rust.contains("__rec_arena.push(TishStruct_TishRecNode"),
        "expected nodes to be bump-allocated into the arena Vec"
    );
}
