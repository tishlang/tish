# Frame-based VM — design & implementation plan (task #39)

The bytecode VM's biggest remaining structural cost. This is a multi-session core rewrite; this doc is
the architecture + sequencing so it can be executed in focused increments behind a flag, flag-off
byte-identical at every step.

## Why — three independent problems, one root cause

The VM calls user functions through `Value::Function(Arc<dyn Fn(&[Value]) -> Value + Send + Sync>)`
(`tish_core/value.rs`). The `Closure` opcode builds one capturing the function's `Chunk` + enclosing
scope; the `Call` opcode does `f(&args)` (`vm.rs:1348`), and that closure **creates a fresh `Vm` and
recursively re-enters `run_chunk`** (`vm.rs:1383`-style). One model, three measured problems:

1. **~275ns / call (the call-overhead wall).** `add2(a,b)` loop-called 20M× = 5501ms — identical at
   arity 2 or 8; the JIT'd body is fast, the call PATH dominates: `Arc<dyn Fn>` indirect dispatch +
   per-call `Vm` struct + `run_chunk` re-entry + arg `Vec`. Only `SelfCall`→native recursion or no-call
   inlined JIT loops escape it. This is the single biggest lever for real numeric code (helpers,
   callbacks, HOFs all pay it).
2. **wasi deep-recursion trap.** `run_chunk` re-entry maps each tish recursion level to a native (and on
   wasi, a *wasm*) call frame. wasmtime exhausts its call stack ~313 levels → `recursion_stress` traps
   ("call stack exhausted"). stacker is a no-op on wasm32 and there is no JIT, so this is unfixable in
   the current model. (See `docs/perf.md` CORRECTNESS FIX note.)
3. **non-numeric recursion overflow + band-aids.** Native-stack recursion forced two band-aids:
   `SelfCall` (JIT native recursion, numeric only) and `stacker::maybe_grow` (VM `vm.rs:1392`, interp
   `eval.rs` `call_func`). Both are mitigations for "recursion lives on the native stack."

Root cause: **calls and recursion live on the native (Rust/wasm) stack.** Fix the model → all three go.

## Target model — `Value::Closure` + an explicit CallFrame stack

- **`Value::Closure(Arc<ClosureData>)`** where `ClosureData = { chunk: Arc<Chunk>, upvalues: Box<[Value]> }`
  (or the existing captured-scope chain). Pure DATA, still `Send + Sync` (Arc<Chunk> is; `Value` is under
  send-values), so it satisfies the load-bearing `Value: Send` constraint. Replaces the opaque
  `Arc<dyn Fn>` — no indirect dispatch, the callee's chunk is reachable directly.
- **One `Vm`, a `Vec<CallFrame>` frame stack.** `CallFrame = { chunk, ip, slot_locals, local_scope,
  enclosing, stack_base }`. `Call` PUSHES a frame and continues the SAME dispatch loop (no new `Vm`, no
  `run_chunk` re-entry); `Return` POPS it and resumes the caller at its saved `ip`. The operand stack is
  shared with per-frame `stack_base` (or one operand stack per frame — decide in step 2).
- Recursion becomes **heap frames**, not native frames → no overflow, wasm-safe, no `maybe_grow` needed,
  and `SelfCall` becomes a pure JIT optimization (not a correctness crutch).

Result: a tish call = push/pop a struct on a `Vec` + a direct chunk switch. No `Arc<dyn Fn>`, no `Vm`
alloc. Expected to collapse most of the 275ns and remove problems 2 + 3 outright.

## The hard part — the builtin↔closure boundary

Builtins call closures DIRECTLY today: `arr.map(cb)` does `f(&args)`; HTTP handlers, `setTimeout`,
Promise reactions all hold a `Value::Function` and invoke it. With `Value::Closure` (data), a builtin
cannot just call it — it needs the VM's frame machinery. Options:

- **(A) Re-entrant `Vm::call_closure(&mut self, closure, args) -> Result<Value>`** that runs a *nested*
  frame loop to completion and returns. Builtins call this. Simple, but builtin→tish callbacks re-enter
  natively (shallow in practice: map/filter callbacks don't deeply nest). Keeps the win for tish→tish
  (the hot path) while accepting native re-entry for the builtin boundary.
- **(B) Hybrid `Value::Function` retained as a thin shim** that captures the VM handle + closure and
  bridges to `call_closure`. Maximizes compatibility (all 104 `Value::Function` sites keep compiling),
  minimizes churn, at the cost of carrying both representations during migration.

Plan: **(A) as the model + (B) as the migration bridge** — land `call_closure`, route builtins through
it, keep a shim so the 104 sites migrate incrementally rather than in one flag-day.

## Flag strategy — `TISH_FRAME_VM`, flag-off byte-identical

Mirror slots/JIT/packed-arrays: gate the new path behind `TISH_FRAME_VM` (default off). Flag-off runs the
current `run_chunk` unchanged (byte-identical, suite stays 17/0). Flag-on runs the frame loop. This makes
the rewrite safe to develop + pause across sessions, and lets the differential harness compare
flag-on ≡ flag-off ≡ interp at every increment.

## Sequencing (each step: compiles, suite 17/0 flag-off, differential flag-on)

1. **`ClosureData` + `Value::Closure`** alongside `Value::Function` (additive; nothing uses it yet).
2. **`CallFrame` + frame stack** in `Vm`; a `run_frames()` loop that handles a SINGLE frame identically
   to `run_chunk` (no calls yet) — prove the loop is equivalent flag-on for call-free programs.
3. **`Call`/`Return`/`SelfCall` as frame push/pop** in `run_frames`; compiler emits `Value::Closure` for
   `Closure` under the flag. Now tish→tish calls + recursion run on frames. Differential vs flag-off on
   the recursion + call corpus (fib, ackermann, recursion_stress, mutual recursion).
4. **`Vm::call_closure` + route builtins** (map/filter/reduce/sort/HTTP/timers/Promise) through it; add
   the `Value::Function` shim for un-migrated sites.
5. **Migrate the 104 `Value::Function` sites** to `Value::Closure`/`call_closure`; drop the shim.
6. **Retire band-aids**: `maybe_grow` (recursion is heap now), and re-evaluate `SelfCall` (keep as JIT
   fast path, drop the VM-recursion variant). Flip `TISH_FRAME_VM` default-on; delete the old path once
   the suite + differential + perf gates pass on all backends (cranelift/wasi/llvm embed the VM → inherit).

## Validation gates (blocking, per increment)

- `cargo test -p tishlang --test integration_test` 17/0 (flag-off AND flag-on).
- Differential `flag-on ≡ flag-off ≡ interp` (timing-normalized) on the call/recursion/HOF/async corpus.
- Perf: `add2` loop-call ≪ 275ns/call (the target); `recursion_stress` completes on **wasi** (the proof
  problem 2 is gone); no regression on the object/array/bundle micros.
- `stacker::maybe_grow` removable without reintroducing the interp/VM overflow.

## Risks

- **Silent miscompile** across all backends (cranelift/wasi/llvm embed the VM) — mitigate with the flag +
  differential harness; never trust, diff.
- **send-values**: `ClosureData` must stay `Send + Sync`; `call_closure` re-entry must not deadlock the
  global-write path under concurrency (see `docs/concurrency-model.md`).
- **Scope/upvalue semantics**: the recurring interp↔vm scope-assignment divergence hazard — port the
  capture model (per-iteration `let`, deep closures via the Vec scope-chain) exactly; differential on
  `closures`, `scopes`, loop-closure fixtures.
- Size: ~900-line loop rewrite + 104 `Value::Function` sites + the builtin boundary. Do it as a focused
  effort, not rushed; the flag makes partial progress safe to bank.
