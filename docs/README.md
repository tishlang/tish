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
| `perf.md` | Perf optimization log: slots/JIT/object layout, HTTP throughput, run-vs-build |
