# A call to a user tish function costs ~92 ticks on GBA

**Status:** item 1 FIXED in `9993d6145` (93.4 -> 72.2 ticks a call). Item 2 PARTLY fixed in
`e85e09566` — the ABI exists and works; it does not yet reach the code that motivated the issue.
Item 3 (below) is what remains.
**Found by:** `tish-gba`, porting a game's rules core from Rust to tish.
**Related:** `int-domain-consumer-boundary.md` — same shape of finding (type information present
and discarded), same bench ROM.

## The measurement

On device, GBA, one frame = 4,389 Timer2 ticks. `chuggie-engine/examples/bench-grid` pass K, 512
iterations of `acc = acc + <read>`:

| | total ticks | per call |
|---|---|---|
| K1 · the read written INLINE | 813 | — |
| K2 · the same read behind `function readCell(i: i32): i32` | 47,691 | **91.6** |

The callee is one line — `return CALLARR[i] & 255` — over a module `i32[]`. The difference is the
call and nothing else. For scale, a typed extern crossing into Rust (`declare fn` + `cargo:` import)
measures about **7 ticks**, so calling a tish function is roughly **13x more expensive than calling
into Rust**.

This is not a microbenchmark curiosity. Counting `value_call` sites in the generated frame loop of
two real ROMs: `drop-tish` 45, `magical-drop` 74. And the number that stopped a port: a game's CPU
search, ported to tish, costs **9,076 ticks per candidate move** — 2.07 frames — against **~100
ticks** for the whole equivalent step in Rust (`bench-grid` pass R2). About twenty cross-module
calls per candidate is most of that gap. The search had to stay in Rust.

## What the generated code does

For `function readCell(i: i32): i32 { return CALLARR[i] & 255 }` called as `mb = mb + readCell(mj)`:

```rust
mb = match &tishlang_runtime::ops::add(&Value::Number(((mb) as f64)), &({
        let _callee = ((readCell).clone()).clone();
        tishlang_runtime::value_call(&_callee, &[Value::Number((mj) as f64).clone()])
    })) { Value::Number(n) => tishlang_runtime::to_int32(*n), _ => panic!("expected number") };
```

and the callee:

```rust
let readCell = {
    let CALLARR_cell = CALLARR_cell.clone();
    Value::native(move |args: &[Value]| {
        let Some(_tish_depth) = tishlang_runtime::enter_call_guarded() else { return Value::Null; };
        let CALLARR = CALLARR_cell.clone();
        let mut i = match &args.get(0).cloned().unwrap_or(Value::Null) {
            Value::Number(n) => tishlang_runtime::to_int32(*n), _ => panic!("expected number") };
        return Value::Number((((...) & 255i32)) as f64);
    })
};
```

**The declared signature is not used at the call site.** `i32` in, `i32` out, and the call still
boxes the argument to `Value::Number` (an i32→f64 soft-float on ARM7TDMI), boxes the return the same
way, and then — because the result's type is unknown — reaches for the fully generic `ops::add`,
which must consider string concatenation, before `to_int32` converts back.

## Item 1 — there is no user-function return-type table

`Statement::FunDecl` carries `return_type`. `collect_native_fns` reads it, for M5 eligibility only.
Nothing maps a user function name to its return type for use at a CALL SITE: there is no
`fn_return_types`, `fn_sigs` or equivalent in `codegen.rs`.

So a call is always an opaque `Value`, and every arithmetic consumer of one falls back to the boxed
path. **The information is present at parse time and thrown away.** That is the same defect shape as
`int-domain-consumer-boundary.md`, where a value was known to be an integer and the consumer
converted it anyway; fixing that one was worth 28x and 4.4x on two hot paths.

Such a table, consulted where a call's result meets a typed consumer, would turn the example above
into an integer add and drop two soft-float conversions per call. It requires no new calling
convention — the call stays boxed, only its RESULT stops being opaque.

**IT CANNOT BE BUILT ON THE ANNOTATION, AND THE FIRST VERSION OF THIS ISSUE SAID IT COULD.** Return
annotations are erased and unchecked:

```tish
function liar(x: i32): i32 {
  if (x > 0) { return "not a number" }
  return 7
}
console.log(liar(1))   // -> not a number
```

That runs, and prints the string. A table keyed on `: i32` would have unboxed it as a number —
either panicking or producing a garbage value where the language today returns a string. The claim
was written from reading the AST and corrected by a two-line program; it is left visible here
because it is the obvious way to build this and it is wrong.

So the table has to be backed by PROOF that every return is numeric, not by the declaration. That
proof already exists: `returns_numeric`, which M5 runs in its fixpoint for exactly this reason.
Reusing it makes the qualification "annotated native return AND every return path verified numeric
AND the name never reassigned" — the same standard M5 already holds itself to, and the reason this
is a contained change rather than a trivial one.

### FIXED in `9993d6145`

    K2 · 512 calls to `function readCell(i: i32): i32`   48,651 -> 37,814 ticks
    per call                                                 93.4 -> 72.2
    a ported CPU search, per step                          28,125 -> 25,464

The call is still a boxed `value_call` — this was never a calling convention. Only its RESULT stops
being opaque, which is enough to keep the surrounding arithmetic integral instead of dropping to
`ops::add` and two soft-float conversions. The remaining ~72 ticks are the dispatch itself, which is
item 2.

Keyed on a PROOF (`collect_user_fn_rets`): every `return` provably numeric, and the name never
reassigned or shadowed. `proof_numeric_type` is separate from `expr_native_type` for the reason
below, which is the second finding this issue produced:

> **`result_type_of_binop` requires BOTH operands to be `F64`.** So `arr[i] & 255` where `arr` is a
> `Vec<i32>` types as `Value` — the element is `I32`, the literal `F64`, and no arm covers the mix.
> On GBA, where `i32[]` is the annotation the hardware wants, that is nearly every masked read of a
> typed array. Widening it would change lowering everywhere it is consulted; the proof asks the
> weaker question instead.

One trap, since it cost a full cycle and left no trace: an emit arm placed at the BOTTOM of
`emit_typed_expr` is DEAD CODE — an existing `Expr::Call` arm shadows it. The measurement does not
move and nothing says why. It belongs beside the typed-extern case inside that arm, which already
does exactly this for `cargo:` imports.

## Item 2 — a MORE precise annotation disqualifies the optimization

M5 (`native_fns`) already lowers a top-level function to a real free `fn`, with direct calls
bypassing `value_call`. Its parameter and return gate is:

```rust
fn ann_is_number(ann: &TypeAnnotation) -> bool {
    RustType::from_annotation(ann) == RustType::F64
}
```

So `: number` qualifies and **`: i32` does not**. On a device with no FPU, `: i32` is the annotation
that is correct for the hardware, and it is the one that opts you out. Every function in the port
that motivated this issue is annotated `: i32`, on the explicit advice of this repo's own GBA
guidance, and every one is therefore ineligible.

Whether M5 should grow an `i32` shape is a design decision rather than an obvious fix — an
`fn f(i32, i32) -> i32` is exactly right for ARM7TDMI, but it is a second native ABI to maintain.

### PARTLY FIXED in `e85e09566`

An all-`i32` signature now emits `fn f_native(mut a: i32, ..) -> i32` and direct calls target it.
All-or-nothing: an `: i32` parameter is ToInt32-coerced on entry through the boxed path and an f64
native parameter is not, so a mixed signature keeps the old ABI.

Two things had to be fixed before the ABI could be reached at all, and both were larger than it:

  * **M5 was switched off entirely on GBA** — the target the annotation exists for. The block was
    gated `emit_mode != Gba` because sibling passes emit `thread_local!`. Those stay off; the
    native-fn pass runs.
  * **`numeric_shaped`/`native_safe_expr` did not know the bitwise operators.** `& | ^ << >> ~` were
    absent from both, so any function containing one was ineligible however it was annotated — and
    on this target that is nearly every function. Now accepted, gated on the GBA numeric vocabulary
    so eligibility elsewhere is byte-identical (ungated it promoted fnv1a and changed its lowering).

And it surfaced a latent miscompile: `native_safe_expr`'s index arm accepted any identifier as the
indexed object without checking it is in scope for a TOP-LEVEL fn, so a fn reading a module array
compiled to a body naming a binding that only exists inside `run()` (E0425). Fixed, with a test.

## Item 3 — a native free fn cannot see module state, which is where the win is

The ABI above changes nothing for the port that motivated this issue, and the reason is exact: every
hot function in it reads a module-scope array (`PLANE[...]`, `DEPTH[...]`, `GS[...]`), and a
module-scope `let` is emitted as a LOCAL OF `run()`, closed over by the boxed function closures. A
top-level `fn` cannot name it — that is the miscompile fixed above, and rejecting those candidates
is correct, not conservative.

So the remaining work is not in the calling convention at all. It is: **emit module-scope arrays and
scalars as top-level statics** (on GBA, where there is a single core and `SingleCore`/`static mut`
are available) rather than as locals of `run()`, so that a native fn can reach them — and then let
`globals` carry them, which every existing check already consults.

Until that lands, a function is eligible only if it touches nothing but its parameters. That is the
minority of any real program, and none of the ~20 calls per candidate in the search this issue
opened with.
Filing it separately from item 1 for that reason.

## What was checked, and what turned out to be wrong

**The annotation is not a contract** — see the `liar` repro under item 1. This is the single most
important thing on this page for anyone implementing it.

**Changing the annotation is not a workaround.** `readCell` was rebuilt as `(i: number): number`:
47,691 → 44,823 ticks, about 6%, and the generated Rust contains **no `readCell_native`** — M5 still
did not fire. The reason is structural and is NOT a bug: an M5 free `fn` cannot see module state,
because the generated program lives inside one `main()` with the module's bindings as locals, and
`readCell` reads a module array. Every function in a game's grid/state modules does. So item 2 alone
would not have helped here; item 1 would.

**Two things in that generated snippet are NOT worth reporting as cost.** `((readCell).clone()).clone()`
is a doubled clone, and `Value::Number(x).clone()` clones a value constructed one expression earlier.
Both are redundant in the emitted source, but `Value::Number` is a non-heap enum variant and LLVM
very likely elides them. They were nearly filed as findings on inspection alone; nobody should act
on them without measuring first.

## Why this shape of program is not served today

The native optimization stack is built around, and benchmarked on, pure numeric functions over
parameters and `f64` arrays — nbody, spectral-norm, fasta, k-nucleotide. Every gap above is
reasonable for that workload.

A GBA game is the opposite shape: `i32` arithmetic, module-scope state arrays that must persist
across frames, and many small cross-module calls per frame. Nothing here is wrong for its intended
target; there is simply no path for this one. The practical consequence is that a tish game's only
lever on call cost is call COUNT, and that lever runs out: batching four per-column loops in the
port bought 38% and the search still did not fit.

## Reproducing

```bash
cd chuggie-engine/examples/bench-grid && npm run build
GBA_SHOT_LOG=1 ../../scripts/screenshot.sh bench-grid.gba /tmp/bg.png 600
```

K1 is the inline baseline, K2 the same work behind a call; the difference over 512 iterations is the
per-call cost. Note the bench's own trap, recorded in its README: adding a function that touches a
module array deoptimises EVERY access to that array, including from top-level code that never calls
it, so K2's callee reads a private array to keep the other passes comparable.
