# Tish Docs (in-repo)

Internal and contributor-facing docs. User-facing docs live in the **[tishlang.com](https://github.com/tishlang/tishlang.com)** repo.

| File | Purpose |
|------|---------|
| `js-emit-philosophy.md` | **Normative:** Tish is not JS; JS emit scope; “obvious failures” vs feature creep; `type` keyword exceptions |
| `ecma-alignment.md` | ECMA-262 / test262 mapping (source of truth; tish-docs summarizes) |
| `LANGUAGE.md` | Canonical language reference (syntax, semantics, builtins; LLM/tool friendly) |
| `plan-gap-analysis.md` | Implementation audit, MVP checklist, next steps |
| `type-system-roadmap.md` | Static type system: status assessment + sequenced roadmap to native, dynamic-free AOT |
| `architecture-next-steps.md` | tish_core refactor, crate layout, design decisions |
| `builtins-gap-analysis.md` | Builtins across Rust vs bytecode VM (Cranelift/WASI) |
| `code-audit-2026-06.md` | Cleanup/optimize/secure audit: what's fixed + prioritized remaining roadmap (DoS limits, interp/core convergence, hot-path allocs) |
| `control-flow-audit.md` | Cross-backend control-flow/scope correctness matrix: 7 pre-existing divergences (let-binding, try-in-fn, switch-break, finally, event-loop) + fix priority. Perf work verified clean |
| `concurrency-model.md` | EXACT task-execution flow (single-thread micro/macrotask + multi-thread HTTP) per backend vs JS/V8: no microtask queue, blocking `await`, prefork+threads+tokio. Why tish deliberately isn't a JS event loop |
| `perf.md` | Perf optimization log: slots/JIT/object layout, mimalloc + parking_lot allocator/lock wins, HTTP throughput, run-vs-build. The dated BEFORE/AFTER blocks at the top are the source of truth |
| `jsc-bun-perf-guidance.md` | JavaScriptCore/Bun techniques as the optimization source: the 4 structural gaps (fat Value, shapeless objects, boxed arrays, one-shot JIT) + ranked roadmap |
| `nan-box-value-plan.md` | Foundational `Value` shrink (24→16→8B): staged plan (abstraction → thin fat variants [safe, independent −33%] → NaN-box swap [unsafe]); the surface (~600 sites) + gates. Implementation is a dedicated workstream |
| `full-backend-parity-plan.md` | Plan to make cranelift/llvm/wasi full-capability (they all package the bytecode VM → fix the VM once); C-ABI `ffi:` native extensions paired across backends (`cargo:` stays rust-only); wasmtime embedder + preview2 for the WASI ecosystem |
| `performance-roi.md` | Tech + ROI synthesis: runtime speedups (startup, compute, HTTP), artifact/runtime size vs Node/V8, and where precompile-then-execute pays off (serverless, containers, CLIs, compute). Marketing-facing; `perf.md`/`concurrency-model.md` are the deep dives |
