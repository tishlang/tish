# Tish Docs (in-repo)

> **Validate — do not trust these numbers.** Any benchmarks, standings, ratios, or
> PASS/acceptance claims below are a point-in-time snapshot and drift the moment the code
> changes — they are illustrative, not ground truth. Re-validate before relying on them:
> `scripts/run_perf_gauntlet.sh` (typed-vs-node PASS/FAIL gate), `scripts/perf_record.sh` +
> `scripts/perf_compare.sh` (over-time, noise-floored), `scripts/run_parity_compare.sh`
> (cross-backend). A verdict means the gate passes **now**, never "we hit X once". Absolute ms
> across different machines/days are not comparable — use a same-machine A/B or the noise-floored
> compare.

Internal and contributor-facing docs. User-facing docs live in the **[tishlang.com](https://github.com/tishlang/tishlang.com)** repo.

| File | Purpose |
|------|---------|
| `js-emit-philosophy.md` | **Normative:** Tish is not JS; JS emit scope; “obvious failures” vs feature creep; `type` keyword exceptions |
| `ecma-alignment.md` | ECMA-262 / test262 mapping (source of truth; tish-docs summarizes) |
| `LANGUAGE.md` | Canonical language reference (syntax, semantics, builtins; LLM/tool friendly) |
| `plan-gap-analysis.md` | Implementation audit, MVP checklist, next steps |
| `type-system-roadmap.md` | Static type system: status assessment + sequenced roadmap to native, dynamic-free AOT |
| `perf-typed-vs-untyped-baseline.md` | **Typed-native A/B gate:** `just perf-gauntlet` (canonical: `scripts/run_perf_gauntlet.sh`) builds each `tests/perf` fixture boxed(flags-off) vs typed(flags-on) vs node, validating that typing speeds programs up without changing results (`TYPED≠BOXED` guard). Speedup ratios are validated on every run, not a recorded state; the doc's table is a snapshot that may be stale — regenerate with `scripts/run_perf_gauntlet.sh`. The snapshot table + what's covered/not |
| `perf-benchmark-suite.md` | **Benchmark-suite map + industry survey:** what tish benchmarks vs what the V8/JSC/SpiderMonkey teams benchmark (SunSpider/Kraken/Octane/JetStream/Are-We-Fast-Yet/Benchmarks-Game), the gap analysis, the 11 canonical algorithmic benchmarks added to `tests/perf` (nbody, mandelbrot, binary_trees, megamorphic, k_nucleotide, …), and the JS-compat gaps porting them surfaced (`>>>`, `1e-3` notation, comma-declarators, `Map.values()` iterators) |
| `architecture-next-steps.md` | tish_core refactor, crate layout, design decisions |
| `builtins-gap-analysis.md` | Builtins across Rust vs bytecode VM (Cranelift/WASI) |
| `code-audit-2026-06.md` | Cleanup/optimize/secure audit: what's fixed + prioritized remaining roadmap (DoS limits, interp/core convergence, hot-path allocs) |
| `control-flow-audit.md` | Cross-backend control-flow/scope correctness matrix: 7 pre-existing divergences (let-binding, try-in-fn, switch-break, finally, event-loop) + fix priority. Perf work verified clean |
| `concurrency-model.md` | EXACT task-execution flow (single-thread micro/macrotask + multi-thread HTTP) per backend vs JS/V8: no microtask queue, blocking `await`, prefork+threads+tokio. Why tish deliberately isn't a JS event loop |
| `http-techempower-status.md` | **HTTP-server health check (snapshot, regenerate with `scripts/run_http_perf.sh` + the doc's regression tests):** records that the default `tiny_http` multithreaded server worked (VM + native AOT, prefork, concurrent, regression tests passed), the `hyper` backend (broken by `#78`: fn_traits `.call`, missing hyper Timer) was fixed, and DB endpoints were blocked by the stale `tish-pg` sibling crate. These are point-in-time states that drift — re-run the reproduce commands rather than trusting the recorded verdict. Verification matrix + reproduce commands |
| `perf.md` | Perf optimization log: slots/JIT/object layout, mimalloc + parking_lot allocator/lock wins, HTTP throughput, run-vs-build. The dated BEFORE/AFTER blocks at the top are the source of truth |
| `jsc-bun-perf-guidance.md` | JavaScriptCore/Bun techniques as the optimization source: the 4 structural gaps (fat Value, shapeless objects, boxed arrays, one-shot JIT) + ranked roadmap |
| `nan-box-value-plan.md` | Foundational `Value` shrink (24→16→8B): staged plan (abstraction → thin fat variants [safe, independent −33%] → NaN-box swap [unsafe]); the surface (~600 sites) + gates. Implementation is a dedicated workstream |
| `frame-vm-plan.md` | Frame-based VM rewrite (task #39): replace per-call `Vm`+recursive `run_chunk` (`Value::Function(Arc<dyn Fn>)`) with `Value::Closure` + an explicit CallFrame stack. Fixes one root cause behind three problems: the ~275ns/call wall, the wasi deep-recursion trap, and native-stack recursion overflow. Flag-gated (`TISH_FRAME_VM`), sequenced, with the builtin↔closure boundary as the hard part |
| `full-backend-parity-plan.md` | Plan to make cranelift/llvm/wasi full-capability (they all package the bytecode VM → fix the VM once); C-ABI `ffi:` native extensions paired across backends (`cargo:` stays rust-only); wasmtime embedder + preview2 for the WASI ecosystem |
| `performance-roi.md` | Tech + ROI synthesis: runtime speedups (startup, compute, HTTP), artifact/runtime size vs Node/V8, and where precompile-then-execute pays off (serverless, containers, CLIs, compute). Marketing-facing; `perf.md`/`concurrency-model.md` are the deep dives |
| `perf-branch-breaking-changes.md` | `feature/perf` branch audit (47 commits vs main): the **default-build breaking changes** — Rust API (`NativeFn`→`Arc<dyn Callable>`, `Value::String`→`ArcStr`, new `NumberArray` variant, `PropMap` struct) and tish-language semantics (div-by-zero→Infinity, insertion-order object keys, ToString conformance, `Array.flat`, `Promise.race`). Almost all language changes are JS-conformance fixes; nothing removed from the surface |
