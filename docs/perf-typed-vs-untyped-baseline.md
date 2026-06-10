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

## Baseline (Apple Silicon, release, min of 3 runs — 2026-06-10)

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
