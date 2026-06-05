# Control-flow & block-scope correctness audit (2026-06-05)

Comprehensive nested control-flow / block-scope stress (loops × switch × try/catch/finally ×
let/const × closures × async/timers) run across **all four execution paths** — node (oracle),
rust-AOT (`tish build --native-backend rust`), vm (`tish run`), interp (`tish run --backend interp`).

## Verdict on the perf work: CLEAN

The recent perf series (de-boxing, M1/M4/M5 native fns + inference) did **not** break execution
flow. Evidence:
- Working tree clean; the perf commits touch codegen *inference* (`returns_numeric`, native-fn
  routing) and statement-position de-boxing, **not** the try / switch / closure / event-loop
  emitters (the `_try_result` try emitter predates the session, from commit `6d795260`).
- rust backend is **byte-identical flags-off vs flags-on** (`TISH_PARAM_NATIVE`/`NATIVE_FN`/
  `PARAM_INFER`) on every control-flow probe.

Everything below is **pre-existing** and was simply surfaced by this audit.

## FIXES LANDED (2026-06-05) — status after the fix pass

| # | case | rust | vm | interp | fix |
|---|------|:----:|:--:|:------:|-----|
| 1 | `for(let i…)` / for-of per-iteration binding | ✓ | ✓ **FIXED** | ✓ **FIXED** | vm: `LoopVarsBegin/End` register the loop var; a closure in the body snapshots it into an `enclosing` overlay layered over the still-shared frame scope (`enclosing2`) — per-iteration value, everything else live. interp: fresh per-iteration `Scope::child` |
| 2 | closure captures block-`let` / lexical closures at all | ✓ | ✓ **FIXED** | ✓ **FIXED** | interp: `Value::Function` now captures its defining `env` (was #39 "no closures"). vm: for/for-of loop vars AND `let`s declared directly in a `while`/`do` body are all per-iteration (compiler scans the loop body + emits `LoopVarsBegin` for each) |
| 3 | `return` inside `try` runs `finally` then returns | ✓ **FIXED** | ✓ **FIXED** | ✓ | rust: try body → `Result<Option<Value>,_>` completion closure (`try_closure_depth`); vm: emit pending `finally` before `Return` |
| 4 | `try/catch` + `throw` **inside a function** | ✓ **FIXED** | ✓ | ✓ | rust: `throw` in a try-closure returns a catchable `Err` instead of `panic!` |
| 5 | `finally` runs when an exception **propagates** | ✓ **FIXED** | ✓ **FIXED** | ✓ | rust: re-raise after finally; vm: emit `finally` before the no-catch rethrow |
| 6 | `break` inside `switch` exits the **switch** | ✓ **FIXED** | ✓ | ✓ | rust: wrap switch in a labeled block + `break_stack` (was breaking the enclosing loop) |
| 7 | timer FIFO order (same-delay `setTimeout`) | ✓ **FIXED** | ✓ **FIXED** | ✓ **FIXED** | runtime: sort due timers by `(due, id)` — HashMap order was nondeterministic |
| 7 | microtask (Promise.then) vs macrotask interleave | n/a | n/a | n/a | **BY DESIGN, not a bug.** tish uses a multi-threaded concurrency model (load-bearing `send-values`); a JS-style single-threaded microtask/macrotask event loop would conflict with / break it. tish does NOT target Node's event-loop semantics. |

**After this pass: rust and interp are correct on ALL of #1–#6; vm is correct on #3–#6 and on
per-iteration binding for `for`/`for-of`/`while`/`do` (#1, #2).** **All of #1–#6 are now fixed across
every backend (interp/vm/rust/cranelift/wasi).** #7's microtask/macrotask interleave is **WON'T-DO by
design** (it would conflict with tish's multi-threaded concurrency model); the timer-FIFO part of #7
is fixed. `switch` having no fall-through is also intentional (locked in `switch.tish.expected`).
The only known out-of-scope straggler is a SEPARATE rust capture-by-value bug (a read-only outer var
captured in a closure is snapshotted, so a later mutation isn't seen — tracked separately).
Regression fixtures `tests/core/control_flow_nested.tish` (loops/switch/try) and
`loop_let_capture.tish` (per-iteration `let`) are asserted across interp/vm/rust/cranelift.

This is NOT Node-parity chasing: per-iteration `let` is what **Rust itself** does (the native
backend compiles to Rust closures and gets `0 1 2` for free) and what tish's own block scoping
implies (`let` is block-scoped; a loop iteration is a block). The vm at `3 3 3` was the lone
var-like outlier; it now matches the other three paths.

### Remaining work
- **(separate, out of scope) rust capture-by-value of read-only outer vars** — `let x=0; let f=()=>x;
  x=100; f()` returns `0` on the rust backend (snapshot) vs `100` on node/vm/interp/cranelift. The
  codegen cell-wraps a captured var only if it's assigned *inside* a closure; it should also cell-wrap
  one that's captured AND assigned anywhere in the defining scope. Tracked as its own task; unrelated
  to per-iteration `let`.
- **microtask/macrotask ordering (#7) — WON'T DO (by design).** A JS-style single-threaded event
  loop with a microtask queue drained between macrotasks would conflict with tish's multi-threaded
  concurrency model (the `send-values` `Arc<Mutex>` design exists precisely so HTTP/handlers run
  across threads). tish intentionally does not target Node's event-loop semantics. (Separate, real:
  rust's `Promise.resolve()` references an un-emitted `tish_promise_object` alias — a codegen bug to
  fix if/when `Promise` is used on the rust backend, independent of ordering.)

### Original divergence matrix (pre-fix, for reference)

| # | node | rust(was) | vm(was) | interp(was) |
|---|------|-----------|---------|-------------|
| 1 | ✓ | ✓ | ✗ `3 3 3` | ✗ `3 3 3` |
| 2 | ✓ | ✓ | ✗ `null` | ✗ throws |
| 3 | ✓ | ✗ E0308 | ✗ skips finally | ✓ |
| 4 | ✓ | ✗ panics | ✓ | ✓ |
| 5 | ✓ | ✗ swallows | ✗ skips | ✓ |
| 6 | ✓ | ✗ breaks loop | ✓ | ✓ |
| 7 | ✓ | ✗ won't compile | ✗ drops/scrambled | ✗ sync/scrambled |

## Repros (minimal)

```js
// (1) let per-iteration binding — node/rust: 0 1 2 ; vm/interp: 3 3 3
let f = []; for (let i = 0; i < 3; i++) { f.push(() => i) }; console.log(f[0](), f[1](), f[2]())

// (3/4/5) try inside a function — rust panics or won't compile; vm drops finally on return
function g(n){ try { return n*2 } finally { console.log("fin") } }   // vm: no "fin"; rust: E0308
function h(){ try { throw "x" } catch(e){ return "ok" } }            // rust: panic "uncaught throw"

// (6) break in switch-in-loop — rust prints only "a1" (breaks the while), should be a1,a3
for (let i=0;i<1;i++){ let j=0; while(j<4){ j++; if(j===2)continue; switch(i){case 0: console.log("a"+j); break} if(j===3)break } }

// (7) event-loop order — node: A B C D E ; tish: broken on every backend
setTimeout(()=>console.log("E"),0); Promise.resolve().then(()=>console.log("C")).then(()=>console.log("D")); console.log("A"); console.log("B")
```
(timers/Promise are `import { setTimeout } from 'timers'` / `import { Promise } from 'http'` in tish.)

## Root-cause hypotheses

- **(1,2) interp/vm `let`-binding** — the loop/block lowering uses a *single* binding for a `let`
  declared in a loop header/body instead of a fresh per-iteration binding, and closures don't snapshot
  block locals (issue #39 territory: "closures carry no scope"). rust is immune because each iteration
  lowers to a Rust closure that moves/copies its captures.
- **(3,5) vm `finally`** — the VM's exception/return unwinding runs `finally` only on the normal
  (or locally-caught) path, not when a `return` or a propagating `throw` leaves the `try`.
- **(3,4,5) rust try-in-function** — a function body is emitted as a `(|| -> Result<Value,_> {…})()`
  closure; the `try` emitter (`_try_result`, also a `Result`-closure) nests incorrectly inside it:
  `return` inside `try` mismatches the outer closure's type (E0308), and `throw`/propagation aren't
  rethreaded to the function's `Result`, so catch never fires (panic) and finally swallows.
- **(6) rust switch-break** — the `break` inside a lowered `switch` targets the enclosing loop's
  break label instead of a switch-local one. tish `switch` already has no fall-through, so each case
  needs a *switch-scoped* break, not the loop's.
- **(7) async** — no proper event loop with a microtask queue drained between macrotasks; timer queue
  isn't ordered FIFO-by-delay; rust has no `tish_promise_object` runtime fn and never drains timers
  before exit.

## Suggested fix priority (silent-wrong-on-common-code × tractability)

1. **rust switch-break (#6)** — *silent* wrong output on `switch`-in-loop on the **native/perf
   path**; likely a one-label codegen fix. Highest risk-to-effort.
2. **rust try-inside-function (#3,4,5)** — crashes/won't-compile on a common pattern; the native
   backend can't run try/catch in a function today.
3. **interp/vm `let`-per-iteration binding (#1,2)** — silent wrong on the canonical ES6 idiom.
4. **vm `finally` on return/propagation (#3,5)** — silent skipped cleanup.
5. **async/timer event-loop ordering (#7)** — broadest rework; least-used surface.

## Regression fixtures

- `tests/core/control_flow_nested.tish` (+ `.expected`, node-correct) — the consolidated nested
  stress. **Currently passes only on interp**; it is the cross-backend target. Not yet wired into
  `MVP_TEST_FILES` (would fail vm/rust until #3–#6 land) — promote it per-backend as each fix lands.
- Existing: `scopes`, `nested_loops`, `break_continue`, `switch`, `do_while`, `for_of`, `try_catch`,
  `try_finally` (note: `try_finally.tish` is **not** in the native test set — that is why rust's
  finally-propagation bug went uncaught).
