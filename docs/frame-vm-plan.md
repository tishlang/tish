# Frame-based VM — design & implementation plan (task #39)

> **Validate — do not trust these numbers.** Any benchmarks, standings, ratios, or
> PASS/acceptance claims below are a point-in-time snapshot and drift the moment the code
> changes — they are illustrative, not ground truth. Re-validate before relying on them:
> `scripts/run_perf_gauntlet.sh` (typed-vs-node PASS/FAIL gate), `scripts/perf_record.sh` +
> `scripts/perf_compare.sh` (over-time, noise-floored), `scripts/run_parity_compare.sh`
> (cross-backend). A verdict means the gate passes **now**, never "we hit X once". Absolute ms
> across different machines/days are not comparable — use a same-machine A/B or the noise-floored
> compare.

The bytecode VM's biggest remaining structural cost. This is a multi-session core rewrite; this doc is
the architecture + sequencing so it can be executed in focused increments behind a flag, flag-off
byte-identical at every step.

## Why — three independent problems, one root cause

The VM calls user functions through `Value::Function(Arc<dyn Fn(&[Value]) -> Value + Send + Sync>)`
(`tish_core/value.rs`). The `Closure` opcode builds one capturing the function's `Chunk` + enclosing
scope; the `Call` opcode does `f(&args)` (`vm.rs:1348`), and that closure **creates a fresh `Vm` and
recursively re-enters `run_chunk`** (`vm.rs:1383`-style). One model, three measured problems (the
numbers below are an **illustrative snapshot — regenerate before citing** with
`scripts/perf_record.sh` + `scripts/perf_compare.sh` for the call micro and
`scripts/run_parity_compare.sh` for the wasi recursion trap; absolute ms are machine/day-specific and
not comparable across runs):

1. **The call-overhead wall (~275ns/call in one snapshot).** `add2(a,b)` loop-called 20M× ≈ 5501ms in
   that snapshot — identical at arity 2 or 8; the JIT'd body is fast, the call PATH dominates:
   `Arc<dyn Fn>` indirect dispatch + per-call `Vm` struct + `run_chunk` re-entry + arg `Vec`. Only
   `SelfCall`→native recursion or no-call inlined JIT loops escape it. This is the single biggest lever
   for real numeric code (helpers, callbacks, HOFs all pay it). Re-measure the per-call cost with a
   same-machine A/B before treating any figure as current.
2. **wasi deep-recursion trap.** `run_chunk` re-entry maps each tish recursion level to a native (and on
   wasi, a *wasm*) call frame. wasmtime exhausts its call stack (~313 levels in one observed run) →
   `recursion_stress` traps ("call stack exhausted"). stacker is a no-op on wasm32 and there is no JIT,
   so this is unfixable in the current model. (See `docs/perf.md` CORRECTNESS FIX note.) The exact
   depth is environment-specific — confirm the trap still reproduces rather than trusting the number.
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
current `run_chunk` unchanged (byte-identical; the integration suite must stay green flag-off — verify
with `cargo test -p tishlang --test integration_test`, don't trust a recorded pass count). Flag-on runs
the frame loop. This makes the rewrite safe to develop + pause across sessions, and lets the
differential harness compare flag-on ≡ flag-off ≡ interp at every increment.

## Sequencing (each step: compiles, integration suite green flag-off, differential flag-on)

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

## Validation gates (blocking, per increment) — criteria + how to check, re-run every time

Each of these is a GATE that must pass **now**, not a state we record once. State the criterion, run the
command, read the result fresh:

- **Suite parity gate:** `cargo test -p tishlang --test integration_test` is green with `TISH_FRAME_VM`
  off AND on (flag-on must not regress any case the flag-off path passes). Validated on every increment
  / in CI, not a frozen count.
- **Differential equivalence gate:** `flag-on ≡ flag-off ≡ interp` (timing-normalized) on the
  call/recursion/HOF/async corpus — check with `scripts/run_parity_compare.sh`. Passing means the
  cross-backend diff is empty on this run, never "it matched once".
- **Call-overhead perf gate:** `add2` loop-call cost is materially below the current baseline (the goal
  is to collapse the call wall, not hit a fixed ns figure) — measure with `scripts/perf_record.sh` +
  `scripts/perf_compare.sh` (same-machine A/B, noise-floored against the JS controls). The typed-vs-node
  standing is gated by `scripts/run_perf_gauntlet.sh` (PASS = typed ≤ node); re-run it, don't cite a
  past PASS.
- **wasi recursion-fix gate:** `recursion_stress` completes (no "call stack exhausted") on the **wasi**
  backend — the proof problem 2 is gone. Confirm by running the fixture on wasi each time, e.g. via the
  cross-backend run in `scripts/run_parity_compare.sh`.
- **No-regression perf gate:** object/array/bundle micros show no regression vs the recorded baseline —
  compare with `scripts/perf_compare.sh` (noise-floored), not eyeballed absolute ms.
- **Band-aid-removal gate:** `stacker::maybe_grow` is removable without reintroducing the interp/VM
  overflow — verify by deleting it and re-running the recursion corpus, not by assumption.

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
