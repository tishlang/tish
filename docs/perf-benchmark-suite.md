# Performance Benchmark Suite — Overview, Industry Survey & Gap Analysis

> **Validate — do not trust these numbers.** Any benchmarks, standings, ratios, or
> PASS/acceptance claims below are a point-in-time snapshot and drift the moment the code
> changes — they are illustrative, not ground truth. Re-validate before relying on them:
> `scripts/run_perf_gauntlet.sh` (typed-vs-node PASS/FAIL gate), `scripts/perf_record.sh` +
> `scripts/perf_compare.sh` (over-time, noise-floored), `scripts/run_parity_compare.sh`
> (cross-backend). A verdict means the gate passes **now**, never "we hit X once". Absolute ms
> across different machines/days are not comparable — use a same-machine A/B or the noise-floored
> compare.

*Last updated 2026-06-10.* This is the map of **what tish benchmarks, what the JavaScript-engine
world benchmarks, and what we were missing**. It complements [`perf.md`](perf.md) (the optimization
log) and [`perf-typed-vs-untyped-baseline.md`](perf-typed-vs-untyped-baseline.md) (the typed-native
A/B). Goal: make sure we are measuring the same things the V8/JSC/SpiderMonkey teams measure, so we
catch missing optimizations instead of overfitting to our own micro-tests.

---

## 1. What we measured *before* this pass

Two harnesses, both real and useful, but both **inward-facing**:

| Harness | Script / location | What it covers |
|---|---|---|
| **Bundled perf suite** | `just perf-suite` → `scripts/run_performance_suite.sh`; fixtures in `tests/core/*.tish` + `tests/modules/*` | Whole-program + per-file **micro-op stress**: `array_stress_01..10` (creation/iteration/map-filter-reduce/sort/search/splice/concat/flat), `object_stress`, `objects_perf`, `string_methods_perf`, `recursion_stress`, `new_features_perf`, `benchmark_granular`. Times vm/interp/rust/cranelift/llvm/wasi vs node/bun/deno/qjs. |
| **Typed-native gauntlet** | `just perf-gauntlet` → `scripts/run_perf_gauntlet.sh`; fixtures in `tests/perf/*.tish` | **A/B**: each fixture built boxed(flags-off) vs typed(flags-on) on the rust backend, timed vs node V8. Pre-pass fixtures: `recursion_fib`, `recursion_untyped`, `matmul`, `math_trig`, `numeric_loop`, `object_sum`, `string_concat`, `array_hof`, `typed_array_hof`. |

**Strengths:** excellent coverage of array/object/string *micro-operations* and a rigorous
typed-vs-untyped validation of the numeric-native codegen.

**The blind spot:** almost no **canonical cross-language / cross-engine algorithmic benchmarks** — the
programs the browser vendors actually use to find optimizations. We had `matmul` and `math_trig` for
float, `fib` for recursion, and that was it. No allocation/GC stress, no megamorphic-dispatch program,
no integer/bitwise kernels, no hashing workload, no JSON round-trip, no backtracking, no PRNG. So we
could not answer "how far are we from V8 on the workloads V8 is tuned for?"

---

## 2. The industry-standard suites (survey)

Surveyed the suites the engine teams use. Status as of 2025–2026:

| Suite | Status | Relevance to a JS-subset |
|---|---|---|
| **SunSpider** | **Retired** (too short, gameable) | Algorithms live on; many absorbed into JetStream as `-SP`. |
| **Kraken** (Mozilla) | **Deprecated** | Very portable (float/DSP/JSON/crypto, no DOM/eval). Good mining ground. |
| **Octane 2.0** (V8) | **Retired 2017** (optimizing it *hurt* real-world perf — Splay→pretenuring, Box2D→false-fold) | Its *algorithms* (Richards, DeltaBlue, Splay, RayTrace, NavierStokes) are the canonical compute microbenchmarks and live inside JetStream. |
| **JetStream 2 / 3** (BrowserBench/WebKit) | **Current** (JS3 shipped Mar 2026) | The standard compute aggregate. JS2's pure-JS members are the best mining ground; JS3 leans into Wasm/async a subset can't run. |
| **Are We Fast Yet** (Marr et al., DLS'16) | **Active** (research) | ★ **Best fit.** *Designed* to be language-core only (objects/closures/arrays/strings; no GC-special, no stdlib, no reflection) so the same program is comparable across languages. All 14 benchmarks are portable. |
| **Computer Language Benchmarks Game** | **Active** | Classic single-core throughput toys; trivially portable (except `pidigits` → BigInt). |
| **Web Tooling / ARES-6** | Folded into JetStream 2 | Too large / feature-heavy (classes, generators) to port cleanly. |
| **Speedometer 3** | Current **web** standard | N/A — DOM/framework responsiveness, not a language-compute benchmark. |

**Key engineering caveat (well-documented by the V8 team):** every *static* benchmark eventually gets
gamed and stops tracking real performance. The value of these suites for us is as **portable
algorithmic workloads that exercise a specific optimization**, not as a score to chase. We run each N
times and the fixtures are sized to resist trivial constant-folding.

### Optimization dimensions each workload stresses

- **Allocation / GC throughput** — binary-trees, Splay, Storage
- **Megamorphic property access / polymorphic dispatch** — Richards, DeltaBlue, *(megamorphic micro)*
- **Float throughput, tight loop, zero hot-path alloc** — mandelbrot, n-body, spectral-norm, NavierStokes
- **Integer / bitwise** — nsieve, fannkuch, crypto (sha/aes), bits-in-byte
- **Hashing / Map** — k-nucleotide, hash-map
- **String building / rope** — fasta, string-concat, base64
- **Regex** — regex-dna, RegExp battery
- **Recursion / backtracking / call overhead** — fib, queens, towers, Ackermann
- **Serialization** — JSON parse/stringify

---

## 3. What was added (this pass)

Eleven canonical benchmarks added to `tests/perf/` — each a **single file valid in both tish and
node** (node runs it directly), self-timed around the kernel, printing
`GAUNTLET <name> <ms> <integer-check>`. The check is an integer so the gauntlet can assert
tish-result == node-result on every backend (catches both correctness regressions and typed-vs-boxed
divergence). **Correctness gate:** `scripts/run_perf_gauntlet.sh` asserts `<integer-check>` matches
node for every fixture on every backend — validated on every run, not a recorded state; a green run
means it passes **now**, not "passed once". Sources: Are We Fast Yet (AWFY) and the Computer Language
Benchmarks Game (CLBG).

| Benchmark | Source | Gap it fills | Stresses |
|---|---|---|---|
| `nbody` | CLBG / AWFY | float + object fields | f64 arithmetic in a hot loop over body objects |
| `mandelbrot` | AWFY / CLBG | tight float loop | scalar f64 inner loop + escape branch, **zero hot-path alloc** (numeric-JIT canary) |
| `spectral_norm` | CLBG / SunSpider | float matrix | nested float loops, division-heavy kernel, array indexing |
| `nsieve` | AWFY / SunSpider | integer + array | large array alloc + strided write/scan + branch prediction |
| `fannkuch` | CLBG / SunSpider | integer permutation | int-array index manipulation, in-place reversal/swaps |
| `binary_trees` | CLBG (GCBench) | **allocation / GC** | many short-lived node objects + recursion (the GC anchor) |
| `queens` | AWFY | backtracking recursion | recursion/call overhead, boolean-array reads/writes, branchy control flow |
| `fasta` | CLBG | PRNG | LCG + cumulative-probability linear search per symbol |
| `k_nucleotide` | CLBG | **Map / hashing** | hash-map insert/lookup at scale (integer-encoded k-mers) |
| `json_roundtrip` | (runtime headline) | **JSON** | `JSON.stringify` + `JSON.parse` of a large nested document |
| `megamorphic` | (V8 elements-kinds) | **megamorphic dispatch** | `.value` read at one site across 8 object shapes (inline-cache/shape signal) |

Still **deliberately not added** (and why): **Richards / DeltaBlue** (the OO macro-benchmarks) — high
value but a large, error-prone port; `megamorphic` is a compact proxy for the dispatch signal until a
faithful Richards lands. **SHA / base64 / AES** — now unblocked (`>>>` landed, see §4) and a good next
add; `fnv_hash` (FNV-1a) is the first of the family. **regex-dna** — tish has `tests/core/regex_perf.js`
but no gauntlet regex program yet; a good next add. **Speedometer** — DOM, N/A.

---

## 4. ★ Compatibility gaps discovered while porting standard benchmarks

Porting real-world JS benchmark code surfaced four tish/JS divergences. **All four have since been
fixed** (across interp/VM/native with parity — see the `tish-js-compat-features` memory note and the
`tests/core/{unsigned_right_shift,scientific_notation,comma_declarators,map_set_iterators}` fixtures);
kept here as the record of how the benchmarks drove conformance work.

1. ~~**Unsigned right shift `>>>` unsupported**~~ → **FIXED.** New `BinOp::UShr`; and the whole bitwise
   family now uses JS ToInt32/ToUint32 (modulo 2³², NaN/±∞→0) instead of a saturating cast, so `>>> 0`
   on a >2³¹ hash is exact. Unblocks the crypto/hashing family; `fnv_hash` is added (a later test pass
   also caught + fixed `Infinity | 0` → was `-1`, now `0`).
2. ~~**Scientific notation `1.66e-3` unsupported**~~ → **FIXED** (lexer). (`nbody` still uses plain
   decimals — harmless.)
3. ~~**Comma-separated declarators `let a = 0, b = 0` unsupported**~~ → **FIXED** (`Statement::Multi`).
4. ~~**`Map.values()`/`keys()`/`entries()` return arrays, not iterators**~~ → **FIXED.** They now return
   real iterators (`{ next() → {value, done} }`) usable via `.next()`, `for…of`, and spread. (Indexing
   `.values()[i]` no longer works — matching JS, where iterators have no `.length`; use `for…of`.)

---

## 5. Initial standings vs V8 (native AOT, `just perf-gauntlet`)

Run `TISH_FAST_NATIVE_BUILD=1 just perf-gauntlet <names…>` — boxed(flags-off) vs typed(flags-on) vs
node V8, min of 2 runs (Apple-silicon, 2026-06-10). `node(ratio)` = typed-on ÷ node; lower is better.
These are **illustrative/machine-specific diagnostics**, not a committed scoreboard — re-run locally.

> **Snapshot — likely stale, regenerate before citing.** The table below is a one-time capture and
> drifts the moment the codegen or backends change. Regenerate with
> `TISH_FAST_NATIVE_BUILD=1 just perf-gauntlet` (wraps `scripts/run_perf_gauntlet.sh`) for the
> typed-vs-node PASS/FAIL and current ratios; absolute ms are machine/day-specific and not comparable
> across runs — use `scripts/perf_record.sh` + `scripts/perf_compare.sh` for a noise-floored A/B.

| benchmark | boxed(off) | typed(on) | typing-speedup | node | ratio vs node |
|---|---|---|---|---|---|
| json_roundtrip | 404ms | 397ms | 1.02× | 133ms | **3.0×** |
| fasta | 224ms | 209ms | 1.07× | 37ms | 5.7× |
| nsieve | 655ms | 492ms | **1.33×** | 72ms | 6.8× |
| queens | 1293ms | 1297ms | 1.00× | 122ms | 10.6× |
| megamorphic | 740ms | 685ms | 1.08× | 56ms | 12.2× |
| mandelbrot | 1052ms | 1036ms | 1.02× | 55ms | 18.8× |
| binary_trees | 960ms | 950ms | 1.01× | 35ms | 27.1× |
| fannkuch | 3929ms | 3906ms | 1.01× | 141ms | 27.7× |
| spectral_norm | 1965ms | 1941ms | 1.01× | 39ms | 49.8× |
| nbody | 908ms | 943ms | 0.96× | 12ms | 78.6× |
| **k_nucleotide** | 16094ms | 15957ms | 1.01× | 9ms | **1773× → 5.6× after the Map fix (Finding 1)** |

Correctness gate (`scripts/run_perf_gauntlet.sh`): typed == boxed == node on every integer check —
validated on every run, not a recorded state. In this snapshot all 11 were **slower than V8** — a
field of gaps to evolve past; re-derive the current standing with the gauntlet rather than trusting
these ratios. Two structural facts jumped out of this snapshot:

1. **The typed-native flags barely move any of them (~1.0×).** The typing work (M1/M4/M5 numeric
   inference) targets scalar/`number[]` kernels; these benchmarks are dominated by **objects, Maps,
   allocation, and megamorphic access**, which that path doesn't touch. The one real mover is `nsieve`
   (1.33×, integer-array writes). This is the empirical confirmation of the
   [`jsc-bun-perf-guidance.md`](jsc-bun-perf-guidance.md) thesis: the next wins are **shapes + inline
   caches, packed arrays, and a tiering JIT** — not more numeric typing.

---

## 6. Findings — what the numbers reveal

### ★★ Finding 1 (headline) — FIXED: `Map`/`Set` were O(n) per operation; now O(1)

In this snapshot `k_nucleotide` was **1773× slower than V8** and the ratio *grew with input size*. A
direct scaling probe (Map insert+lookup of N entries) confirmed the original super-linear cost, and the
fix. The table below is a point-in-time capture (regenerate the current `k_nucleotide` ratio with
`just perf-gauntlet k_nucleotide`); the *shape* of the result — flat O(1)/op after the fix vs
quadrupling-per-doubling before — is the durable claim, the absolute ms are stale-able:

| N | node | tish *before* (O(n)/op) | tish *after* (O(1)/op) |
|---|---|---|---|
| 20,000 | 2ms | 505ms | 5ms |
| 40,000 | 3ms | 2026ms (**4.0×**/doubling) | 10ms |
| 80,000 | 5ms | 8166ms (4.0×) | 23ms (**~2.2×**/doubling) |

**Before:** every `Map.get/set/has` was a **linear scan** — `tish_builtins/src/collections.rs` backed
`Map` with two parallel `Vec<Value>` and did `keys.iter().position(|e| same_value_zero(e, &key))` per op;
`Set` likewise. Quadrupling-per-doubling ⇒ O(n)/op ⇒ O(n²) for n ops.

**Fix (done):** both `Map` and `Set` are now backed by a single insertion-ordered hash map
(`indexmap::IndexMap<Key, Value>`). `Key` wraps a `Value` with a `Hash`+`Eq` that implements
**SameValueZero** — number/string/bool/null keys (the common case) hash by value → true O(1); reference
keys hash by a per-variant tag, disambiguated by `ptr_eq`. `delete` uses `shift_remove` to preserve
iteration order. The `.size` hook (`SizeProbe`/`collection_size`/`size_probe_len`) and constructors keep
their signatures, so **no interp/VM/native edits were needed** — `tests/core/set_map_types.*` stays
byte-identical on all backends (interp/vm/native verified) and `cargo test -p tishlang_builtins` is green.
Result: at n=80k, **8166ms → 23ms (~350×)**; `k_nucleotide` **1773× → 5.6× vs V8** (now dominated by
general object/boxing overhead, Finding 2 — not the Map).

### Finding 2: object-field access in float loops is the largest gap (nbody, ~80× in snapshot)

`nbody` (78.6× in the §5 snapshot) and `megamorphic` (12.2×) isolate object property access: every field read goes
through an `Arc<Mutex<PropMap>>` with no shape/inline-cache on the native path. Maps to jsc-bun gap #1
(shapeless objects) — the same root cause as the Map issue (no hashing/shape specialization).

### Finding 3: the scalar-float inner loop isn't lowered to a native f64 loop

`mandelbrot` (18.8×), `spectral_norm` (49.8×), `fannkuch` (27.7×) — §5 snapshot ratios — are tight
numeric loops where typing
gives ~1.0×. The values stay boxed `Value` through the loop; this is the numeric-JIT / OSR ceiling
(jsc-bun gap #3, region JIT) — already on the roadmap, now measurable.

### Finding 4: allocation/GC throughput is a major gap (binary_trees, ~27× in snapshot)

Every tree node is a heap `Arc<Mutex<…>>` object; V8 bump-allocates + generational-GCs. Maps to the
`Value`-size / object-layout work ([`nan-box-value-plan.md`](nan-box-value-plan.md)).

**The meta-point:** these benchmarks are **diagnostics, not a scoreboard**. When a number here improves
after an optimization, the optimization generalizes beyond our own micro-tests. When it doesn't move,
we overfit. Finding 1 alone justified the exercise.

---

## 7. Recommended next steps

1. **~~Fix `Map`/`Set` O(n)-per-op~~ ✅ DONE** — re-backed with `indexmap::IndexMap` (Finding 1); now
   O(1)/op, ~350× faster at n=80k, all backends byte-identical.
2. **Shapes + inline caches for objects** (jsc-bun gap #1) → addresses `nbody`/`megamorphic`/
   `binary_trees` (and is the same "no hashing/specialization" root cause as #1).
3. **~~Fix `>>>` + sci-notation + comma-declarators~~ ✅ DONE** — all landed across backends; `fnv_hash`
   added (32-bit integer/bitwise dimension now measurable). Next in the bitwise/hashing family: SHA-1 /
   base64. **Open perf item it exposed:** the JIT bails on shifts, so hashing loops run on the VM
   interpreter (`fnv_hash` ~33× V8 in the §5-era snapshot — recheck with `just perf-gauntlet fnv_hash`);
teaching the JIT `<<`/`>>`/`>>>` would close most of it.
4. **Port Richards + DeltaBlue** (AWFY versions, prototype-style) → the canonical megamorphic-dispatch
   macro-benchmarks; the single best signal for object/IC work.
5. **Add a regex gauntlet program** (regex-dna) → wire the existing regex workload into the vs-node A/B.
6. **Consider a JetStream-style aggregate score** (geomean over the suite) so a single number tracks
   progress, while keeping the per-benchmark diagnostics.
