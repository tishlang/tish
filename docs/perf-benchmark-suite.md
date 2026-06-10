# Performance Benchmark Suite — Overview, Industry Survey & Gap Analysis

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
divergence). Sources: Are We Fast Yet (AWFY) and the Computer Language Benchmarks Game (CLBG).

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
faithful Richards lands. **Crypto / SHA / base64** — blocked on a parser gap (`>>>`, see §4).
**regex-dna** — tish has `tests/core/regex_perf.js` but no gauntlet regex program yet; a good next add.
**Speedometer** — DOM, N/A.

---

## 4. ★ Compatibility gaps discovered while porting standard benchmarks

Porting real-world JS benchmark code surfaced four tish/JS divergences. The first three are **parser
gaps** (valid JS that tish rejects); the fourth is a **documented semantic deviation**. These are
worth tracking independently of perf — they are exactly the kind of "obvious failure" the JS-emit
philosophy says we should close.

1. **Unsigned right shift `>>>` is unsupported.** The lexer reads `a >>> b` as three `>` (`Gt`) tokens
   and the parser errors. Blocks the entire crypto/hashing family (SHA-1/MD5/AES, base64) which relies
   on `>>> 0` / `>>> n` for 32-bit-unsigned semantics. `>>`, `<<`, `&`, `|`, `^` all work.
2. **Scientific / exponent notation `1.66e-3` is unsupported.** The lexer stops the number at `1.66`
   and treats `e` as an identifier ("Undefined variable: e"). Worked around in `nbody` by writing the
   constants as plain decimals.
3. **Comma-separated declarators `let a = 0, b = 0` are unsupported** ("Expected Comma, got Ident").
   Split into separate `let` statements.
4. **`Map.values()` / `keys()` / `entries()` return arrays, not iterators** (a documented deviation).
   Indexing `.values()[i]` works on tish but is `undefined` on node (iterators have no `.length`), so
   it silently diverges. **Portable idiom: `for (const v of m.values())`** — iterates identically on
   both. (Used in `k_nucleotide`.)

None block the benchmarks above (worked around), but **(1) blocks a whole benchmark category** and is
the highest-value fix if we want crypto/hashing coverage.

---

## 5. Initial standings vs V8 (native AOT, `just perf-gauntlet`)

Run `TISH_FAST_NATIVE_BUILD=1 just perf-gauntlet <names…>` — boxed(flags-off) vs typed(flags-on) vs
node V8, min of 2 runs (Apple-silicon, 2026-06-10). `node(ratio)` = typed-on ÷ node; lower is better.
These are **illustrative/machine-specific diagnostics**, not a committed scoreboard — re-run locally.

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

All 11 are correctness-clean (typed == boxed == node on every check); all are **slower than V8** — a
field of gaps to evolve past. Two structural facts jump out:

1. **The typed-native flags barely move any of them (~1.0×).** The typing work (M1/M4/M5 numeric
   inference) targets scalar/`number[]` kernels; these benchmarks are dominated by **objects, Maps,
   allocation, and megamorphic access**, which that path doesn't touch. The one real mover is `nsieve`
   (1.33×, integer-array writes). This is the empirical confirmation of the
   [`jsc-bun-perf-guidance.md`](jsc-bun-perf-guidance.md) thesis: the next wins are **shapes + inline
   caches, packed arrays, and a tiering JIT** — not more numeric typing.

---

## 6. Findings — what the numbers reveal

### ★★ Finding 1 (headline) — FIXED: `Map`/`Set` were O(n) per operation; now O(1)

`k_nucleotide` was **1773× slower than V8** and the ratio *grew with input size*. A direct scaling
probe (Map insert+lookup of N entries) confirmed the original super-linear cost, and the fix:

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

### Finding 2: object-field access in float loops is ~80× off (nbody)

`nbody` (78.6×) and `megamorphic` (12.2×) isolate object property access: every field read goes
through an `Arc<Mutex<PropMap>>` with no shape/inline-cache on the native path. Maps to jsc-bun gap #1
(shapeless objects) — the same root cause as the Map issue (no hashing/shape specialization).

### Finding 3: the scalar-float inner loop isn't lowered to a native f64 loop

`mandelbrot` (18.8×), `spectral_norm` (49.8×), `fannkuch` (27.7×) are tight numeric loops where typing
gives ~1.0×. The values stay boxed `Value` through the loop; this is the numeric-JIT / OSR ceiling
(jsc-bun gap #3, region JIT) — already on the roadmap, now measurable.

### Finding 4: allocation/GC throughput is ~27× off (binary_trees)

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
3. **Fix `>>>`** (parser) → unlocks the crypto/hashing benchmark family (SHA-1, MD5, AES, base64) — a
   whole optimization dimension (32-bit integer/bitwise) we currently can't measure.
4. **Port Richards + DeltaBlue** (AWFY versions, prototype-style) → the canonical megamorphic-dispatch
   macro-benchmarks; the single best signal for object/IC work.
5. **Add a regex gauntlet program** (regex-dna) → wire the existing regex workload into the vs-node A/B.
6. **Consider a JetStream-style aggregate score** (geomean over the suite) so a single number tracks
   progress, while keeping the per-benchmark diagnostics.
7. **Add scientific-notation + comma-declarator parsing** → removes friction porting any future
   standard benchmark.
