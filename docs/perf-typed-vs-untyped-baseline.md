# Typed-native perf baseline (typed vs untyped A/B)

**What this is.** The committed validation that the typed-native codegen (the "typing work") actually
makes programs faster — and never changes their result. Each compute benchmark in `tests/perf/` is
built **twice on the rust backend** and self-timed (hot loop only; process startup excluded):

| build | flags | path exercised |
|-------|-------|----------------|
| **boxed (off)** | all typing flags **unset** | the dynamic `Value` path — the untyped baseline |
| **typed (on)**  | all typing flags **set** | native `f64` / `Vec<f64>` / `String` / structs / native fns |

`typing-speedup = boxed / typed` is the win attributable to the typing work. Both are also compared to
Node (V8). Run it with:

```bash
cargo build --release -p tishlang --bin tish
just perf-gauntlet            # or: ./scripts/run_perf_gauntlet.sh --runs 3
```

The flag set lives in `scripts/run_perf_gauntlet.sh` (`TYPED_FLAGS`) and must stay in lockstep with
`docs/type-system-roadmap.md`: `TISH_PARAM_NATIVE` (M1), `TISH_PARAM_INFER` (M4), `TISH_NATIVE_FN`
(M5), `TISH_STRUCT_INFER` (struct/array), `TISH_FUSED_HOF` (fused reduce), `TISH_NATIVE_HOF` (native
`number[]` HOFs).

## Update 2026-06-10 — phase 1: M4 inference widening (idiomatic numeric code goes native)

The gauntlet grew to 21 compute benchmarks (the canonical Benchmarks-Game / Are-We-Fast-Yet set).
Profiling them surfaced that the **codegen was already V8-competitive** for scalar numeric code, but
the *inference* wasn't reaching idiomatic functions. Three small, fully-flag-gated changes to
`tish_compile/src/infer.rs` (dark-ship intact — flags-off byte-identical):

1. **`collect_numeric_locals` now counts base-inferred `let i = 0`** (number-literal initializers), not
   just annotated `let i: number`. This is what lets `i < n` prove the *param* `n` numeric.
2. **`nus_expr` recurses through logical `&&`/`||`** (a param used numerically *inside* a condition,
   `iter < maxIter && x*x+y*y <= 4`, is a numeric use) and treats the **bitwise/shift family** as
   unambiguously numeric.
3. **M4 (`param_infer_program`) now runs BEFORE local/struct inference** — so derived numeric locals
   (`let x0 = (px/w)*3`) get typed off the now-known param types instead of falling back to boxed
   `Value` and dragging the whole hot loop boxed with them. (This ordering was the keystone: with it,
   an *unannotated* numeric function lowers exactly like an annotated one.)

**Result — `mandelbrot` 872 ms → 70 ms (12.46× typing-speedup), from 13.9× *off* V8 to 1.21× off.**
`fnv_hash` also fully nativizes (was un-runnable before `>>>`); its remaining 4× gap vs V8 is an
*integer-representation* issue (it round-trips `f64↔i32` every bitwise op via `to_int32`, where V8
keeps the value an int32) — a separate future lever, not boxing. **Soundness held: 0 `TYPED≠BOXED`,
0 `≠NODE` across all 21.** Also landed: **OOB-safe typed array reads** (`vec.get(i).unwrap_or(NaN/false)`
for numeric/bool `Vec`s — JS `arr[oob]` is `undefined`, not a panic), hardening the rest-param path
and the foundation for inferring index reads.

| benchmark | boxed (off) | typed (on) | typing-speedup | node (ratio) | status |
|-----------|------------:|-----------:|---------------:|-------------:|--------|
| `object_sum` | 88 ms | 1 ms | **87.91×** | 3 ms (0.33×) | PASS ✓ |
| `array_hof` | 315 ms | 15 ms | **21.00×** | 42 ms (0.36×) | PASS ✓ |
| `matmul` | 231 ms | 14 ms | **16.50×** | 17 ms (0.82×) | PASS ✓ |
| `recursion_fib` | 459 ms | 29 ms | **15.83×** | 54 ms (0.54×) | PASS ✓ |
| `recursion_untyped` | 453 ms | 32 ms | **14.16×** | 51 ms (0.63×) | PASS ✓ |
| **`mandelbrot`** | 872 ms | **70 ms** | **12.46×** | 58 ms (1.21×) | FAIL (was 13.9× off → now near V8) |
| `typed_array_hof` | 264 ms | 96 ms | 2.75× | 33 ms (2.91×) | FAIL (evolve) |
| `nsieve` | 306 ms | 308 ms | 0.99× | 61 ms (5.05×) | FAIL — needs mutable typed arrays |
| `fnv_hash` | 494 ms | 512 ms | 0.96× | 122 ms (4.20×) | FAIL — native; needs i32-local repr |
| `spectral_norm` | 1842 ms | 1709 ms | 1.08× | 39 ms (43.8×) | FAIL — needs typed array params |
| `fannkuch` | 3673 ms | 3995 ms | 0.92× | 146 ms (27.4×) | FAIL — mutable int arrays |
| `queens` | 1051 ms | 1070 ms | 0.98× | 119 ms (8.99×) | FAIL — mutable int arrays |
| `nbody` | 848 ms | 865 ms | 0.98× | 11 ms (78.6×) | FAIL — array of structs |
| `binary_trees` | 1023 ms | 1033 ms | 0.99× | 43 ms (24.0×) | FAIL — recursive struct alloc |
| `megamorphic` | 686 ms | 685 ms | 1.00× | 55 ms (12.5×) | FAIL — polymorphic dispatch (VM IC) |
| `mandel/matmul/recursion/object_sum/array_hof` are the PASS wins above. `numeric_loop`/`math_trig`/`string_concat` are neutral-by-design (already native / memory-bound). |

### Remaining levers (sequenced; each leverages typed, all gated by the gauntlet `TYPED≠BOXED` guard)

- **#2 Mutable typed arrays (`Vec<f64>`/`Vec<i32>`/`Vec<bool>`).** Targets `nsieve`/`fannkuch`/`queens`
  (local arrays) and, with typed array params, `spectral_norm`. Needs: (a) ✅ OOB-safe **read** (done);
  (b) OOB-safe **index-assign** — `{ let _i = i as usize; if _i >= v.len() { v.resize(_i+1, <NaN/false>); } v[_i] = x; }`
  (JS sparse-grow); (c) **element-type inference from `push`** (`let a = []; a.push(1.0)` → `number[]`),
  not just array literals; (d) a **mutable-array-safe** use analysis (allow index read/write/push/length,
  bail on escape/foreign methods); (e) typed **array params** (`fn f(v: number[])` → `&mut Vec<f64>` for
  mutation — the spectral_norm case). Code sites: `infer.rs` `si_block`/`uses_are_array_safe`/`infer_array_elem`;
  `codegen.rs` typed `Index` (~6651) + `IndexAssign` emit + `.push`/`.length`.
- **#3 Tighter native HOF/fold.** `typed_array_hof` 2.91× off — inline the reducer to cut per-element
  closure dispatch; pair with packed `Float64Array`.
- **#4 Arrays of structs (`Vec<Struct>`).** `nbody` (78× off) — extend `TISH_STRUCT_INFER` to
  arrays-of-objects with native field access in loops.
- **#5 Native recursive struct allocation.** `binary_trees` (24× off) — `Box`/`Rc` node structs vs boxed
  `PropMap`; the hardest (recursive nullable `{left,right}`).
- **(orthogonal) i32-local representation** for pure-int hash loops (`fnv_hash`), and feeding M4's
  inference into the **VM JIT** (the default `tish run` path).

## Update 2026-06-10 — phase 2: mutable typed arrays (local `number[]` / `boolean[]`)

Lever **#2** (the local-array half) landed. A `let a = []` (or `[lits]`) that is only ever
**index-read / index-assigned / `.push`ed / `.length`ed / `for…of`-iterated** — never escaping to a
boxed context — now lowers to a native `Vec<f64>` / `Vec<bool>` instead of a boxed `Value::Array`.
Three pieces, all still fully flag-gated (dark-ship intact):

1. **OOB-safe index-assign** (`codegen.rs` `IndexAssign`): JS `a[i] = x` past the end *grows* the
   array (holes read back as `undefined`), it does not panic — so for `Vec<f64>`/`Vec<bool>` we emit
   `{ let _idx = i; if _idx >= v.len() { v.resize(_idx+1, <NaN/false>); } v[_idx] = x; }`. In-bounds
   is the same direct store. (Reads were already OOB-safe via `.get().unwrap_or(NaN/false)` in phase 1.)
2. **Element-type inference from `push`** (`infer.rs` `infer_expr_type` gained `Index`→elem and the
   bitwise family→number), so `let a = []; a.push(1.0)` infers `number[]` even with no array literal.
3. **A block-level verified-hypothesis fixpoint** (`block_native_arrays`): collect every top-level
   `let X = []` candidate, hypothesize *all* are `elem[]`, verify each (every value written/pushed is
   provably `elem` under the hypothesis, and the array never escapes — bare ident reference, foreign
   method, reassignment, or return all bail), drop failures, repeat until stable. The stable set is
   self-consistent, hence **sound**. Run once per element type (`number[]`, then `boolean[]`). This is
   what handles cross-referencing arrays a per-array pass can't — `perm[i] = perm1[i]` (fannkuch) only
   types if *both* `perm` and `perm1` survive together.

**Results (full 21-benchmark gauntlet, min of 3, idle machine). Soundness held: 0 `TYPED≠BOXED`,
0 `≠NODE`.**

| benchmark | boxed (off) | typed (on) | typing-speedup | node (ratio) | element type | note |
|-----------|------------:|-----------:|---------------:|-------------:|----|----|
| **`nsieve`** | 467 ms | **102 ms** | **4.58×** | 70 ms (1.46×) | `Vec<bool>` | was 5.88× off V8 → 1.46× |
| **`fannkuch`** | 3532 ms | **1405 ms** | **2.51×** | 143 ms (9.83×) | `Vec<f64>` | cross-ref arrays (`perm`/`perm1`) |
| `queens` | 1079 ms | 1077 ms | 1.00× | 123 ms (8.76×) | — (escapes) | arrays passed to `place()` → correctly stays boxed |
| `spectral_norm` | 1820 ms | 1753 ms | 1.04× | 39 ms (44.95×) | — (escapes) | arrays passed to `multiply*()` → needs array **params** |

`queens`/`spectral_norm` are the **escape** cases: their arrays are created locally but passed as
arguments to other functions, so the local-array analysis correctly bails (the value `42600` /
`norm` is byte-identical to boxed and node — sound, just not yet faster). They need the cross-function
**typed-array-params** sub-lever below. A soundness probe (`[1,2,3]` summed alongside a *string* array
`["x","y"]`) stays boxed for the string array and prints `6xy` identically typed vs node — the
analysis provably can't store a string into a `number[]`.

### Updated remaining levers (post phase 2)

- **#2b Typed array *params* (cross-function).** The biggest remaining single target —
  `spectral_norm` (45× off) and `queens` (8.8× off), plus most of the Benchmarks-Game suite. Needs an
  **M4-analog whole-program fixpoint**: a param (or local) is a native f64/bool array iff every use is
  array-safe *or* it is forwarded to another position that is itself a native-array param. Then codegen
  emits `&Vec<f64>` (read-only) / `&mut Vec<f64>` (written — mutability inferred) params, threads
  `&x`/`&mut x` (or reborrows `&**p`) at callsites, and **bails on aliasing** (same ident passed to two
  array params in one call, which Rust's borrow checker would reject). Conceptually clear; the codegen
  borrow-threading tail is the careful part.
- **#3 Tighter native HOF — already largely done.** The native `reduce` *already* lowers to a tight
  native loop (`{ let mut acc = init; for x in v.iter().copied() { acc = body; } acc }`) — no closure
  dispatch, no boxing. `typed_array_hof`'s residual 3.0× gap is its body `(a*31+b) % 1000003` emitting
  f64 `fmul`/`frem` where V8 uses **integer** ops. So #3 **collapses into the i32-representation
  lever** (shared with `fnv_hash`), not closure inlining.
- **i32-local representation (shared: `fnv_hash` 4.1×, `typed_array_hof` 3.0×).** Detect integer-valued
  numeric locals/reducers and emit `i32`/`i64` arithmetic instead of `f64` (round-tripping through
  `to_int32` per bitwise op, and `frem` for `%`, is the cost). Needs sound range modelling (or i53) —
  the highest-reward orthogonal lever now that it covers two benchmarks.
- **#4 Arrays of structs (`Vec<Struct>`).** `nbody` (79× off). **#5 Native recursive struct alloc.**
  `binary_trees` (24× off, hardest). Both extend `TISH_STRUCT_INFER`. `megamorphic` (12× off) is
  polymorphic dispatch → a VM inline-cache concern, not typed codegen.

## Baseline (Apple Silicon, release, min of 3 runs — pre-phase-1 reference, 9-benchmark set)

| benchmark | boxed (off) | typed (on) | typing-speedup | node (ratio) | status | validates |
|-----------|------------:|-----------:|---------------:|-------------:|--------|-----------|
| `object_sum` | 90 ms | 2 ms | **44.98×** | 3 ms (0.67×) | PASS | struct inference — unboxed field access |
| `array_hof` | 245 ms | 12 ms | **20.41×** | 30 ms (0.40×) | PASS | fused reduce over a boxed array (`TISH_FUSED_HOF`) |
| `matmul` | 234 ms | 15 ms | **15.60×** | 16 ms (0.94×) | PASS | M1 annotated params → native `f64` indexing |
| `recursion_untyped` | 462 ms | 31 ms | **14.90×** | 55 ms (0.56×) | PASS | M4+M5 inference — idiomatic *untyped* code goes native |
| `recursion_fib` | 467 ms | 32 ms | **14.59×** | 54 ms (0.59×) | PASS | M1 native param + M5 native monomorphic call |
| `typed_array_hof` | 268 ms | 100 ms | **2.68×** | 34 ms (2.94×) | FAIL (evolve) | native `number[]` (`Vec<f64>`) reduce (`TISH_NATIVE_HOF`) |
| `numeric_loop` | 47 ms | 50 ms | 0.94× | 53 ms (0.94×) | PASS | already native via base codegen (memory-bound) |
| `math_trig` | 12 ms | 12 ms | 1.00× | 81 ms (0.15×) | PASS | `Math.*` intrinsics already native |
| `string_concat` | 0 ms | 0 ms | — | 3 ms | PASS | too fast to measure either way |

**8 / 9 typed-native builds beat V8.** Numbers are indicative (single machine, min-of-3); re-run the
gauntlet for your hardware.

## What the baseline shows

- **The typing work pays off where it should — compute-heavy, dispatch-bound code:** 14–45× on
  recursion, matmul, struct sums, and HOF reductions. The dominant cost it removes is the boxed
  `value_call` ABI and per-element `Value` boxing.
- **Inference reaches idiomatic code:** `recursion_untyped` (no annotations) gets the *same ~15×* as
  the annotated `recursion_fib`, because M4/M5 infer `n: number` and the numeric return and emit a
  native `fib_native`. You do not have to annotate to get the win on numeric code.
- **Neutral cases are expected, not regressions:** `numeric_loop` (0.94×) and `math_trig` (1.0×) are
  already native through the base typed codegen, and a trivial loop is memory-bandwidth-bound, so the
  flags add nothing. `0.94×` is run-to-run noise (47 vs 50 ms), not a real slowdown.
- **Soundness is validated for free:** the gauntlet flags `TYPED≠BOXED` if the typed and boxed builds
  ever disagree on a result. **No fixture triggered it** — the dark-shipped flags are behavior-
  preserving across this corpus (consistent with the byte-identical cross-backend corpus).
- **`typed_array_hof` is an honest open gap.** The native `Vec<f64>` reduce (`TISH_NATIVE_HOF`) gives
  a real **2.68×** over the boxed `array_reduce`, but the remaining native fold is still ~2.9× slower
  than V8's JIT on this hash-fold workload — so it's the one `FAIL (evolve)` row. This is the gauntlet
  working as intended: it tracks red→green. The packed-native `Float64Array` follow-up and better fold
  codegen are the levers (see `type-system-roadmap.md`).

## Scope / what this does NOT cover yet

- **Not a CI gate.** Perf fixtures are timing-nondeterministic (excluded from the parity corpus) and
  run on demand via `just perf-*`. A typing change that silently *stopped* helping is caught by eye
  on the next gauntlet run, not automatically.
- **The runtime stdlib types** (`Date`/`Set`/`Map`/typed arrays) are correctness features, not
  typed-vs-untyped speedups — they behave identically with flags on or off, so they have no row here.
  Their validation is the cross-backend + Node parity corpus (`tests/core/{date_types,set_map_types,
  typed_arrays}.*`), not this baseline.
- **One machine, indicative numbers.** Treat the absolute ms as a snapshot; the *ratios* (and the
  PASS/FAIL/TYPED≠BOXED verdicts) are the durable signal.

## Adding a fixture

Drop `tests/perf/<name>.tish` that self-times its hot loop and prints
`GAUNTLET <name> <elapsed_ms> <check>` (the `<check>` is a result value compared across boxed / typed
/ node). If the `.tish` uses type annotations (so node can't run it), add a type-erased
`tests/perf/<name>.js` twin. The gauntlet picks it up automatically.
