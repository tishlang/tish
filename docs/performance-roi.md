# Tish Performance & ROI: precompile-then-execute vs a JS runtime (V8/Node)

> **Validate — do not trust these numbers.** Any benchmarks, standings, ratios, or
> PASS/acceptance claims below are a point-in-time snapshot and drift the moment the code
> changes — they are illustrative, not ground truth. Re-validate before relying on them:
> `scripts/run_perf_gauntlet.sh` (typed-vs-node PASS/FAIL gate), `scripts/perf_record.sh` +
> `scripts/perf_compare.sh` (over-time, noise-floored), `scripts/run_parity_compare.sh`
> (cross-backend). A verdict means the gate passes **now**, never "we hit X once". Absolute ms
> across different machines/days are not comparable — use a same-machine A/B or the noise-floored
> compare.

> **Status:** technical + marketing synthesis, 2026-06-05. Every speed figure cites
> [`perf.md`](perf.md) (darwin-arm64, release builds, measured). Binary/artifact sizes are measured
> locally on darwin-arm64 and are **indicative**, not a published benchmark. The deep dives are
> [`perf.md`](perf.md) (methodology, full tables) and [`concurrency-model.md`](concurrency-model.md)
> (execution model). This doc answers *what you save and where* — not *how it works internally*.

---

## TL;DR

Tish lets you **precompile** TypeScript/JavaScript-shaped source ahead of time and **ship a
self-contained artifact** — instead of shipping source plus a ~112 MB V8 runtime that re-parses,
re-compiles, and re-warms its JIT on every cold start.

The figures in this table are a **snapshot that may be stale — regenerate the speed rows with
`scripts/run_perf_gauntlet.sh` (PASS/FAIL gate) and `scripts/perf_record.sh` +
`scripts/perf_compare.sh` (noise-floored), the HTTP row with `scripts/run_http_perf.sh`.** Absolute
ms are not comparable across machines/days; treat the deltas as illustrative, not a current verdict.

| Dimension | Tish | Node / V8 | Delta (snapshot) |
|---|---|---|---|
| **Cold start** | ~12 ms | ~35 ms | **~3× faster** ([`perf.md:16`](perf.md)) |
| **Core suite avg (47 tests, total)** | 576 ms (native) | 1412 ms | **~2.45× faster** ([`perf.md:975`](perf.md)) |
| **Compute kernels** | 6 of 8 beat V8 | baseline | up to **6.8× faster** ([`perf.md:187`](perf.md)) |
| **HTTP, single worker** | 125k req/s, 1.02 ms p50 | 95k, 1.35 ms | **+33% req/s** ([`perf.md:87`](perf.md)) |
| **Shipped artifact** | ~5 MB static binary | source + 112 MB runtime + node_modules | **runtime-free** |
| **Container image** | ~5–10 MB (`FROM scratch`) | ~150 MB+ base + deps | **~15–30× smaller** |

**The honest ceiling:** object/dynamic-heavy code is still **~2× slower** than V8 (its hidden
classes make property access nearly free), and only the **rust** backend gets the wins above — the
cranelift/llvm backends run at VM speed. See [Where it doesn't pay (yet)](#5-where-it-doesnt-pay-yet).

---

## 1. Runtime speed

### 1.1 Startup — the most universal win

Tish cold-starts in **~12 ms** vs Node's **~35 ms** ([`perf.md:16`](perf.md)). There is no source
to parse, no bytecode to compile, and no JIT to warm up at launch — a precompiled artifact is ready
on instruction one. **46 startup-bound tests in the suite all beat Node** for exactly this reason.

This dominates any workload that starts a fresh process often: CLIs, serverless functions, cron
jobs, autoscaling replicas, and short-lived scripts. A V8 process that lives 50 ms and exits never
reaches its optimizing JIT — it runs interpreted/baseline the whole time. Tish's AOT native path is
already optimized before the first request.

### 1.2 Whole-suite average

Across the 47-test core suite (release, [`perf.md:975`](perf.md)). These standings are a
**snapshot that may be stale — regenerate with `scripts/perf_record.sh` +
`scripts/perf_compare.sh`** (noise-floored over time); the ordering is not a settled verdict:

| Engine | Total (47 tests) | vs Node (snapshot) |
|---|---|---|
| **tish — native (rust)** | **576 ms** | **2.45× faster** |
| QuickJS | 579 ms | 2.44× |
| tish — bytecode VM | 815 ms | 1.73× |
| Bun | 669 ms | 2.11× |
| Deno | 1218 ms | 1.16× |
| **Node (V8)** | 1412 ms | 1.0 (baseline) |

The public landing figure is **12 ms vs 30 ms average (~2.5×)**. **Be honest about what this
average is:** most core tests are tiny, so the suite average is **startup-dominated** — it largely
restates §1.1. The numbers that reflect *compute* are in §1.3.

### 1.3 Compute kernels — where AOT meets or beats V8

The perf gauntlet ([`perf.md:187`](perf.md)) runs compute-only benchmarks (process startup
excluded) on the **rust backend** vs Node/V8, each with a correctness check.

**Acceptance gate, not a recorded score:** `scripts/run_perf_gauntlet.sh` reports per-kernel
PASS when typed tish <= node and FAIL otherwise; it is validated on every run (CI/release), not a
state we "hit once". The table below is a **snapshot — regenerate with
`scripts/run_perf_gauntlet.sh`** (it may be stale; the gate's current verdict is whatever that
command prints today):

| Kernel | Tish | Node | Result (snapshot) |
|---|---|---|---|
| math_trig | 12 ms | 82 ms | ✅ **6.8× faster** (native Math intrinsics → f64) |
| recursion_untyped | 31 ms | 51 ms | ✅ **1.6×** (param + return type inference → native call) |
| recursion_fib(35) | 31 ms | 48 ms | ✅ **1.5×** (monomorphic native calls) |
| numeric_loop (40M) | 44 ms | 47 ms | ✅ beats V8 (statement-position de-boxing) |
| matmul 256² | 14 ms | 16 ms | ✅ beats V8 (native scalar params) |
| string_concat | 3 ms | 3 ms | ✅ parity (`s = s + x` → `push_str`, O(1)) |
| array_hof (reduce) | 108 ms | 29 ms | ❌ 3.7× slower (needs packed f64 arrays) |
| object_sum | 11 ms | 3 ms | ❌ 3.7× slower (needs native struct arithmetic) |

How those wins were won, for the skeptical reader:
- **De-boxing** — a 40M-iteration numeric loop went 111 → **48 ms**, beating Node's 52 ms, by not
  constructing+dropping a dead boxed `Value` per statement ([`perf.md:121`](perf.md)).
- **Native scalar params** — real `fn bench(N: number)` matmul went 301 → **15 ms** (20×, and 3×
  faster than Node's 45 ms) by binding a native f64 shadow at function entry ([`perf.md:133`](perf.md)).
- **Monomorphic native calls** — `fib(35)` went 512 → **31 ms** (beats Node 48) by emitting a
  parallel `fib_native(f64) -> f64` and calling it directly instead of boxing every argument
  ([`perf.md:153`](perf.md)).
- **Numeric JIT** — array `find/some/every` numeric callbacks went 96 → **7 ms** (13.7×) by
  compiling straight-line f64 callbacks to native code via Cranelift ([`perf.md:9`](perf.md)). This
  one rides along on the VM, so the bytecode/cranelift/llvm/wasi paths benefit too.

### 1.4 HTTP throughput

Single-worker, native rust server, `oha -c128`, darwin-arm64 ([`perf.md:87`](perf.md)). These
numbers are a **snapshot — regenerate with `scripts/run_http_perf.sh`** (same-machine A/B vs node);
absolute req/s drift with machine and load:

| Engine | /plaintext | /json |
|---|---|---|
| **tish** w=1 | **125k req/s · 1.02 ms p50** | **121k · 1.05 ms** |
| node w=1 | 95k req/s · 1.35 ms p50 | 93k · 1.38 ms |

In this snapshot, apples-to-apples, **tish served ~33% more requests per worker** (native server +
cached `Date` header + `Arc<str>` bodies) — re-confirm with `scripts/run_http_perf.sh` before
quoting it; it is not a standing guarantee. Multi-core scaling comes from prefork + OS threads (§3), not a single
event loop. *(Honest platform note: macOS `SO_REUSEPORT` doesn't kernel-load-balance, so tish's
multi-worker row doesn't scale **on macOS** and Node's `cluster` overtakes it there — a platform
artifact. Prefork scaling is real on the Linux deployment target; [`perf.md:94`](perf.md).)*

---

## 2. Size & footprint

### 2.1 Toolchain (install once, at dev/CI time)

| Tool | Size (darwin-arm64) |
|---|---|
| **tish** (full toolchain: lexer→parser→VM→native backends) | **14 MB** |
| Node v24.8 | 112 MB |
| Bun | 57 MB |
| Deno | 153 MB |

### 2.2 Shipped artifact (what actually goes to production)

This is the number that matters for deployment, and where precompile-then-execute pays:

| Artifact | Size | Needs a runtime installed? |
|---|---|---|
| Native binary (rust backend), e.g. `hello` | **~4.8–6.4 MB** | **No** — statically links the runtime |
| WASI module (embeds bytecode + VM) | **~791 KB** | Any wasm runtime (wasmtime, etc.) |
| WASM runtime (browser) | **~599 KB** | The browser |
| Node service | your JS (KBs) **+ 112 MB Node + node_modules** | **Yes**, on every host |

A trivial program compiles to ~4.8 MB because it statically links `tish_runtime` — not tiny, but
**runtime-free**: nothing else needs to be present on the target machine. There is no Node, no Rust,
no `node_modules`.

### 2.3 Container image — the punchline

Because the artifact is a single static binary, the image is the binary plus (almost) nothing:

- **Tish:** `FROM scratch` (musl static build) or distroless-static + a ~5 MB binary ≈ **5–10 MB**.
- **Node:** a `node:slim`/`node:alpine` base (~120–200 MB) **+ your app + node_modules**.

That's commonly a **~15–30× smaller image**, which compounds into real operational ROI: faster
registry pulls, lower egress/storage cost, faster autoscale spin-up, and a much smaller CVE/attack
surface (no package tree shipped to production).

---

## 3. Precompile-then-execute vs V8 (the model, and where the cost moves)

| | **Ship-to-V8 (Node)** | **Precompile-then-execute (Tish)** |
|---|---|---|
| What you ship | source `.js` + 112 MB runtime + deps | one self-contained artifact (~5 MB) |
| At startup | parse → compile to bytecode (Ignition) **every time** | nothing — artifact runs directly |
| Reaching peak speed | JIT (Sparkplug→TurboFan) warms up **after** hot code runs | rust backend is AOT-optimized from instruction one |
| Short-lived process | dies before the optimizing JIT engages | already optimal |
| Where the compile cost lives | **every cold start**, on every host | **once**, at build time in CI |
| Runtime dependency | V8 must be installed/bundled | none (static binary) |

Tish offers three precompiled forms ([`LANGUAGE.md`](LANGUAGE.md)):

1. **Native, rust backend** — emits Rust, compiles to a native binary; typed/inferred numeric paths
   lower to real native f64 (this is the backend that beats V8 in §1.3).
2. **Native, cranelift/llvm backend** — embeds serialized bytecode + the VM in a binary; throughput
   is **VM-class**, not AOT-native (Cranelift only builds the object wrapper; [`LANGUAGE.md:104`](LANGUAGE.md)).
3. **Portable bytecode / WASM / WASI** — small, embeddable artifacts that run the same VM anywhere.

The stated direction is to lower more of the language to primitives (`Vec<f64>`, fixed layouts) and
add real bytecode→IR AOT for hot paths ([`LANGUAGE.md:110`](LANGUAGE.md)) — i.e. the de-boxing wins
in §1.3 are slice one of a longer roadmap, not the ceiling.

**Concurrency is part of the model**, not an afterthought: Tish uses real multi-core parallelism
(prefork processes ⊕ OS threads ⊕ per-thread tokio), where Node uses a single event loop plus opt-in
Worker threads. The trade is deliberate — Tish gives up V8's single-thread micro/macrotask ordering
to get genuine parallelism. Full detail and the rationale: [`concurrency-model.md`](concurrency-model.md).

---

## 4. Where the ROI is

**Serverless / edge / FaaS.** Cold start ~3× faster (12 vs 35 ms) **and** AOT means there is no JIT
warmup to miss — a function that runs for tens of milliseconds runs at full speed, where V8 would
still be interpreting. Add a ~5 MB artifact and you cut both p99 latency and GB-s billing.

**Containers & microservices.** ~15–30× smaller images → faster pulls, cheaper registry/egress,
faster autoscale, smaller attack surface. A `FROM scratch` service image is realistic.

**CLIs & developer tools.** ~3× faster startup multiplied across thousands of invocations is
felt directly, and you ship one binary — your users don't need Node installed.

**Compute / numeric services.** On the rust backend, most compute kernels meet or beat V8 in the
last snapshot (math ~6.8×, recursion ~1.5–1.6×, numeric loops and matmul at/above parity) — the
current count and per-kernel verdict is whatever `scripts/run_perf_gauntlet.sh` reports now. There is no tracing GC
in the host toolchain and no stop-the-world pause, which tightens tail latency for steady workloads.

**Multi-core servers.** +33% req/s per worker, and real parallelism across cores via prefork +
threads (scales on Linux) — without rewriting your code around Worker message-passing.

**Security & supply chain.** I/O is feature-gated (`http`/`fs`/`process` are opt-in), the artifact
is a single memory-safe-hosted static binary, and there is no `node_modules` tree in production to
audit or get compromised.

---

## 5. Where it doesn't pay (yet)

A credible perf story states its ceilings. These are real and current:

- **Object/dynamic-heavy code stays ~2× Node.** V8's hidden classes compile property access to
  nearly free; Tish doesn't have that yet (in the last snapshot `object_sum` and `array_hof` were
  the gauntlet's FAIL kernels, ~3.7× slower; [`perf.md:28`](perf.md)). Which kernels currently FAIL
  is whatever `scripts/run_perf_gauntlet.sh` reports now — verify rather than assume this list. If
  your hot path is property-bag manipulation, V8 still wins.
- **Only the rust backend gets the §1.3 wins.** The cranelift/llvm/wasi paths run at **VM speed**,
  and non-JIT inline loops/recursion/array-index on the VM are ~120× slower than Node until a
  baseline/loop JIT lands ([`perf.md:56`](perf.md)). Choose the rust backend for compute-bound work.
- **There's a build step.** The rust backend invokes `cargo`; the first build is slow and CI needs a
  Rust toolchain. You trade per-cold-start compile (V8) for per-CI-build compile (Tish).
- **It isn't full JavaScript.** No `class`/prototypes, no `==`, strict-equality only, no
  micro/macrotask ordering, `setTimeout` is best-effort outside the interpreter, and the builtin/npm
  surface is a subset. This is a migration cost, not a drop-in Node replacement — see
  [`concurrency-model.md`](concurrency-model.md) and [`LANGUAGE.md`](LANGUAGE.md).

**Net:** Tish wins decisively on **startup, size, deployment, and numeric compute**, is at parity-ish
on straight-line code, and is behind on **dynamic/object-heavy** workloads. Pick it where those first
four dominate — which is most serverless, edge, CLI, container, and compute-service work.

---

## 6. Reproduce / further reading

- Acceptance gate (typed vs node PASS/FAIL): `scripts/run_perf_gauntlet.sh` — the source of truth
  for every "beats V8" / PASS claim above; runs in CI/release, validated each run.
- Speed over time (noise-floored vs JS controls): `scripts/perf_record.sh` then
  `scripts/perf_compare.sh` — use this for the suite/standings rows, not a frozen number.
- HTTP throughput (same-machine A/B): `scripts/run_http_perf.sh`.
- Cross-backend parity: `scripts/run_parity_compare.sh`.
- Broader suite (informational): `./scripts/run_performance_suite.sh --release`, `just perf-gauntlet`, `just perf-http`.
- Sizes: `tish build app.tish -o app && ls -lh app` (vs `ls -lhL $(which node)`).
- Deep dives: [`perf.md`](perf.md) (full tables, methodology), [`concurrency-model.md`](concurrency-model.md)
  (execution model vs V8), [`LANGUAGE.md`](LANGUAGE.md) (backends, compile targets, direction).
