# Recursive struct native codegen (#178) — design + status

**Goal:** make recursive `{left,right}`-style data structures (binary_trees and any developer code
with the same *shape*, under arbitrary identifiers) compile to native Rust structs with
`Option<Box<T>>` children and native typed-fn recursion — instead of boxed `Value` closures +
`object_from_pairs` + `get_prop` hash lookups.

This is the real fix that retires the fixture-name `binary_trees_check` kernel (Category A). It is
**name-independent by construction** (keys on structure, not identifiers) and **flag-gated**
(`TISH_REC_STRUCT`, off by default) so nothing existing changes until it's proven.

## Validated baseline (2026-06-21)

Renamed binary_trees (buildNode/sumNode/runTrees, nodes `{left,right}`), all typed flags on:
- **typed ~785ms vs node ~35ms (~22×)**.
- Generated Rust: functions are dynamic `VmRef<Value>` closures called via `value_call`; nodes are
  `Value::object_from_pairs([("left",..),("right",..)])`; field reads via `get_prop(&node,"left")`.
- The #177 aggregate machinery does NOT engage: it requires a `type A = {..}` alias used as `A[]`
  (linear `Vec<A>`), and `alias_is_copy_struct` rejects non-Copy (recursive) fields. So a parallel,
  self-contained recursive-struct pass is the right vehicle — not an extension of the array path.

## Key finding: `Box` is necessary but NOT sufficient — the win is an ARENA

First cut used `Option<Box<Node>>` (one `malloc` per node). Measured on a depth-22 binary tree (8.4M
nodes), renamed identifiers, all typed flags on:

| variant | min ms | vs node |
|---|---|---|
| boxed `Value` (today) | ~1970 | 15× slower |
| **`Option<Box<Node>>`** | **~1960** | **still 15× slower** |
| node (V8) | ~129 | 1× |

`Box`-per-node is **no faster than the boxed `Value` path** — both are bound by per-node heap
alloc/free, which V8's generational GC (bump-allocate + cheap young-gen sweep) crushes. The real fix
is an **arena**: all nodes in one bump-allocated `Vec`, children referenced by `i32` index (`-1` =
null), `Copy` nodes. One big allocation, no per-node `malloc`, freed in one shot.

| variant | min ms | vs node |
|---|---|---|
| **arena (`Vec<Node>` + i32 indices)** | **~37** | **3.5× FASTER** |

So this pass ships the **arena** lowering. (`RustType::Boxed` is kept in the type system for future
non-arena / shared recursive shapes, but the tree path uses indices.)

## Target lowering (arena)

```rust
#[derive(Clone, Copy, Debug, Default)]
struct TishStruct_TishRecNode { left: i32, right: i32 } // -1 = null; scalar fields would be f64

fn buildNode_rec(d: f64, __rec_arena: &mut Vec<TishStruct_TishRecNode>) -> i32 {
    if d > 0.0 {
        let __rec_c_left = buildNode_rec(d - 1.0, __rec_arena);   // children first (stable indices)
        let __rec_c_right = buildNode_rec(d - 1.0, __rec_arena);
        let __rec_idx_new = __rec_arena.len() as i32;
        __rec_arena.push(TishStruct_TishRecNode { left: __rec_c_left, right: __rec_c_right });
        return __rec_idx_new;
    }
    let __rec_idx_new = __rec_arena.len() as i32;
    __rec_arena.push(TishStruct_TishRecNode { left: -1, right: -1 });
    return __rec_idx_new;
}

fn sumNode_rec(__rec_idx: i32, __rec_arena: &Vec<TishStruct_TishRecNode>) -> f64 {
    let __rec_n = __rec_arena[__rec_idx as usize];   // Copy out — no borrow held across recursion
    if __rec_n.left == -1 { return 1.0; }
    1.0 + sumNode_rec(__rec_n.left, __rec_arena) + sumNode_rec(__rec_n.right, __rec_arena)
}

// top level: build into a fresh arena, consume by root index, free the whole arena at block end
let r = { let mut __rec_arena = Vec::new();
          let __rec_root = buildNode_rec(15.0, &mut __rec_arena);
          sumNode_rec(__rec_root, &__rec_arena) };
```

## Pieces

1. **`RustType::Boxed(Box<RustType>)`** — renders `Box<...>`; recursive child fields are
   `Option(Box::new(Boxed(Box::new(Named))))` → `Option<Box<TishRec_Node>>`. (Required: a struct
   containing `Option<Self>` without Box is infinitely sized.)
2. **Detection (`detect_recursive_struct_program`)** — structural, name-independent:
   - A *builder* fn: every return is an object literal with a fixed key set; each field value is a
     scalar, `null`, or a recursive self-call (→ child of the same struct).
   - A *consumer* fn: param is the node; body reads node fields, null-checks (`x.left === null`),
     recurses on child fields, returns numeric.
   - Synthesize one struct shape unifying the field sets; child fields = `Option<Box<Self>>`.
3. **Struct decl** — register the synthesized struct so `emit_named_struct_decls` emits it (with the
   Boxed field rendering).
4. **Native fn emission** — builder (`-> Node`), consumer (`&Node -> f64`), orchestrator (`f64 -> f64`
   calling both); recursion direct; struct literals → `Node { f: Some(Box::new(child)) | None }`;
   field access direct; `x.left === null` → `x.left.is_none()`; child arg → `x.left.as_ref().unwrap()`.
5. **Call routing** — top-level / orchestrator calls dispatch to the native fns, bypassing `value_call`.
6. **Gate** — `TISH_REC_STRUCT=1`. Off ⇒ identical to today.

## Status

- [x] Validated baseline + decided parallel-pass architecture.
- [x] `RustType::Boxed` foundation (kept for future shared/non-arena recursive shapes).
- [x] Detection (structural, name-independent) + arena struct decl.
- [x] Native builder + consumer fn emission (arena, i32 indices, `Copy` nodes).
- [x] Top-level `consumer(builder(literals))` call routing + `TISH_REC_STRUCT` gate.
- [x] **Validated (core shape)**: renamed binary-tree build+count == node checksum; **~37ms vs node
      ~129ms (3.5× faster), ~53× over boxed**.
- [x] **Loop-bearing orchestrator** (`binaryTrees`): native numeric-fn body emitter that holds a
      node-index (`i32`) local (`longLived`), `<<` shift, while/for loops, assignment/inc, and calls
      builders (→ index) / consumers (→ f64) threading the arena. Top-level orchestrator call sets up
      a fresh arena. **Validated on the FULL renamed AND actual binary_trees fixture: ~28–37ms ≈ node
      ~37ms, checksum 6444382 matches, NO `binary_trees_check` kernel** — the honest, name-independent
      path. With both `TISH_NATIVE_FN` (fusion) and `TISH_REC_STRUCT` on, rec takes precedence and
      output stays correct.
- Regression tests: `tests/perf_codegen_178_rec.rs` (core composition + orchestrator).
- [ ] **Per-tree arena reset (nursery)** — currently the arena grows to the run's total node count
      (~6M / ~50MB for `binaryTrees(15)`, freed at block end). Fine for the fixture; for large N or
      long-running servers, truncate the arena to a per-iteration checkpoint for throwaway trees
      (escape analysis: only reset trees that don't outlive the iteration; `longLived` must survive).
- [ ] Then: default-gate review (retire the fusion kernel once rec is in the gauntlet `TYPED_FLAGS`),
      broader shapes (multiple structs, scalar+child mix, consumer extra params).

### Safety model (why this can't miscompile)

- Off unless `TISH_REC_STRUCT=1`.
- Boxed closures are **left intact** as a fallback; native fns are emitted with scratch-buffer
  rollback (any unsupported construct disables the whole path).
- Routing only fires for fully-native-emittable calls; everything else uses the unchanged boxed path.

Tracked by #203 / #178. Companion plan: `.cursor/plans/perf_work_audit_*.plan.md`.
Set `TISH_REC_DEBUG=1` to trace detection/emission decisions.
