# tish concurrency & task-execution model (vs JS / V8)

**Status:** descriptive (how it works today), 2026-06. Source-grounded; file:line references throughout.

## TL;DR

tish is **not** a single-threaded event-loop language and does not try to be. There is **no microtask
queue anywhere in the codebase** (the word "microtask" appears once, in a stale comment at
`crates/tish_eval/src/promise.rs:2`). `await` is a **blocking** operation on every backend, not a
coroutine suspension. Concurrency comes from **OS processes + OS threads + tokio**, not from one
reactor multiplexing callbacks.

| | **V8 / Node** | **tish** |
|---|---|---|
| Core concurrency unit | one thread + event loop | OS **processes** (prefork) + OS **threads** + tokio runtimes |
| `await` | suspends the coroutine, returns to the loop | **blocks the OS thread** (`block_on` / channel `recv`) |
| Microtask queue (Promise jobs) | global FIFO, drained after every task & callback | **none** |
| Macrotask/timer queue | libuv phases, keeps process alive | a `thread_local` `HashMap`, drained opportunistically |
| "microtask before macrotask" guarantee | yes | **no such ordering exists** |
| `async function` | returns a Promise; body is a state machine | flag **ignored** — compiled as an ordinary sync function |
| CPU parallelism | Worker threads (separate isolates, message-passing) | **native** — handlers run in parallel across threads/processes |

The rest of this document is the *exact* flow.

---

## 1. The three backends have three different task models

There is no shared async machinery. Each execution backend has its own Promise representation and its
own (or no) timer drain:

| Backend | Promise impl | Timer drain | `await` |
|---|---|---|---|
| **interpreter** (`tish_eval`) | `tish_eval/src/promise.rs` — stateful, per-promise reaction queue | `run_timer_phase()` — a real keep-alive loop | `block_on(oneshot)` |
| **bytecode VM** (`tish_vm`) | `tishlang_runtime::promise` — lazy `block_until_settled` chains | `drain_timers()` **once** at program exit | `AwaitPromise` op → `block_until_settled` |
| **rust-native** (`tish_compile`) | same `tishlang_runtime::promise` | **none emitted** (fires only via blocking I/O) | `await_promise(...)` → `block_until_settled` |

This per-backend divergence is itself a difference from V8, where there is exactly one event loop with
one well-defined ordering.

---

## 2. Single-threaded execution flow (a plain script)

### 2a. V8 / Node (for reference)

```
run synchronous script to completion (call stack empties)
loop:
  drain the ENTIRE microtask queue (Promise .then/.catch/.finally jobs, queueMicrotask)
  run ONE macrotask (a timer callback, an I/O callback) from the current libuv phase
  drain the ENTIRE microtask queue again
  ...repeat until both queues empty; the process stays alive while any timer/handle is pending
```

Two guarantees that define JS async: (1) a `.then` callback **never** runs synchronously — it is
always deferred to the microtask queue; (2) **all** microtasks drain before the **next** macrotask.

### 2b. tish — synchronous phase

All three backends run the top-level statements straight through, synchronously, to the end. This part
matches JS (the call stack). The difference is entirely in *what happens to deferred work*.

### 2c. tish — Promises (what JS calls microtasks)

**No microtask queue exists.** A Promise is, at the core, just "a thing you can block on":
`tishlang_core::Value::Promise(Arc<dyn TishPromise>)` where `trait TishPromise { fn
block_until_settled(&self) -> Result<Value, Value>; }` (`crates/tish_core/src/value.rs:109,297`).

- **VM / rust-native** (`crates/tish_runtime/src/promise.rs`): `.then(cb)` is **lazy**. It does not run
  `cb` and does not queue it — it builds a `ThenPromise` that *captures* `cb` and returns immediately
  (`promise.rs:220`). The callback runs **only if and when someone awaits** the resulting promise:
  `ThenPromise::block_until_settled` (`promise.rs:78-97`) blocks on the predecessor, *then* calls the
  handler. A `.then` chain is a linked list collapsed bottom-up at await time. `new Promise(executor)`
  runs the executor **synchronously** and returns an `mpsc`-backed promise you block on
  (`promise.rs:165-197`). `Promise.all` blocks on each element **sequentially** (`promise.rs:118`);
  there is **no `.finally`** on this path (`vm.rs:2185` returns "not found").
  → **Consequence:** a `.then` whose result is never awaited **never runs**. Its callback is silently
  dropped.

- **interpreter** (`crates/tish_eval/src/promise.rs`): the only stateful model — `PromiseState` is
  `Pending { reactions: VecDeque<Reaction> } | Fulfilled(v) | Rejected(e)`. `.then(cb)`:
  if the predecessor is **already settled**, `cb` runs **synchronously, right then**
  (`eval.rs:2752-2790`); if **pending**, `cb` is queued as a `Reaction` and flushed **synchronously
  inside the resolver call** when the promise later settles (`eval.rs:2510-2560`). This per-promise
  `VecDeque<Reaction>` is the closest thing to a microtask queue in the tree — but it is **per-promise,
  not a global FIFO**, and it runs **eagerly/synchronously**, never deferred.
  → **Consequence:** `.then` on an already-resolved promise runs *before* the next synchronous line —
  the opposite of JS, where it always defers.

In **neither** backend is there a phase that drains pending Promise jobs after the script. JS's
"microtasks drain before the program continues" simply does not happen.

### 2d. tish — timers (what JS calls macrotasks)

`setTimeout`/`setInterval` register into a `thread_local! REGISTRY: HashMap<u64, TimerEntry>`
(`crates/tish_runtime/src/timers.rs:17-26`; the interpreter has its own clone in
`tish_eval/src/timers.rs`). When they actually **fire** differs sharply per backend — this is the
single biggest behavioral split:

- **interpreter** — a real keep-alive loop. `run_timer_phase()` (`eval.rs:2845-2877`, called after the
  script from `lib.rs:40`) loops `while has_pending_timers()`, **sleeping** until the next due instant
  and running due callbacks (`MAX_ITERATIONS = 1_000_000` as a runaway guard). `setTimeout(fn, 100)`
  in a plain script **fires** — the process waits for it, like Node.

- **bytecode VM** — `drain_timers()` runs **exactly once**, at the end of `Vm::run` (`vm.rs:1820-1823`).
  It only fires timers already due (`due <= now`); there is no sleep-until-next. So `setTimeout(fn, 0)`
  fires (it's due by the time the script ends), but `setTimeout(fn, 100)` in a fast script **likely
  never fires** — nothing keeps the program alive to 100ms.

- **rust-native** — **no drain is emitted at all**. The generated `run()` returns without draining
  (`codegen.rs` emits a bare `Ok(())`). Timers fire **only opportunistically** if the program enters a
  blocking op that calls `sleep_with_drain` (today only `ws.receiveTimeout`, `ws.rs:145`). A
  `setTimeout(fn, 0)` in a plain compiled binary with no blocking I/O **never fires**.

Ordering of same-delay timers: the runtime registry sorts due timers by `(due, id)` — deterministic
FIFO (`timers.rs:68-88`, a recent fix; the HashMap order was nondeterministic). The interpreter's
registry is **not** sorted (a known divergence). Nested timers (a timer scheduling a timer) drain up to
**64 generations** per `drain_timers` on the VM/native path (`run_due_timers`, `timers.rs:51-66`); the
interpreter chains them via its keep-alive loop instead.

### 2e. Worked example — the canonical micro/macro test

```js
console.log("A sync")
setTimeout(() => console.log("E timer0"), 0)
Promise.resolve().then(() => console.log("C micro1")).then(() => console.log("D micro2"))
console.log("B sync")
```

| | output | why |
|---|---|---|
| **V8/Node** | `A B C D E` | sync A,B → drain microtasks C,D → macrotask E |
| **interp** | `A C D B E` | `Promise.resolve()` is settled, so `.then(C)`/`.then(D)` run **synchronously during the chain** (before B); timers fire after in `run_timer_phase` |
| **vm** | `A B E` (C,D dropped) | `.then` is lazy and nothing awaits the chain → C,D **never run**; the 0 ms timer fires at the single exit drain |
| **rust-native** | won't compile unless `async`; when async, the Promise path is incompletely wired and a 0 ms timer never drains | `tish_promise_object`/`await_promise` are only `use`d in async mode (`codegen.rs:1339`); no exit timer drain |

There is no backend, and no ordering, under which tish reproduces V8's `A B C D E`. This is **by
design** — see §4.

### 2f. `async` / `await`

`await` **blocks the current OS thread** on every backend — it does not suspend a coroutine:
- interp: `block_on(oneshot rx)` via a thread-local tokio runtime (`promise.rs:158-179`).
- VM: `Opcode::AwaitPromise` → `p.block_until_settled()` (`vm.rs:1736-1763`); rejections rethrow.
- rust-native: `await x` compiles to `tish_await_promise(x)` → `block_until_settled` (`codegen.rs:3604`).

The `async` keyword on a user function is **ignored**: bodies are compiled/evaluated as ordinary
**synchronous** closures (`codegen.rs:2572-2573`, `compiler.rs:983`, `eval.rs:522-540`). An `async`
function that doesn't `await` runs fully synchronously and its return value is **not** auto-wrapped in
a Promise. Only the top-level `run()` of a native binary is a real `async fn`, driven by
`#[tokio::main]` (`codegen.rs:1372`) — a shell, not user-level suspension.

---

## 3. Multi-threaded execution flow (HTTP serving)

This is where tish's model diverges most from JS — and where the design pays off. `serve(port,
handler)` is **genuinely parallel across cores**, with no single-threaded bottleneck.

### 3a. `send-values`: the feature that makes it legal

`VmRef<T>` is the interior-mutability cell behind every `Value::Array`/`Object`/`RegExp`
(`crates/tish_core/src/vmref.rs`):
- default: `Rc<RefCell<T>>` — `!Send` (interp, wasm, cranelift/llvm).
- with `send-values`: `Arc<Mutex<T>>` — `Send + Sync` (`vmref.rs:111-162`), and `NativeFn` becomes
  `Arc<dyn Fn + Send + Sync>` (`value.rs:68-71`).

The `http` feature **forces `send-values` on** (`tish_vm/Cargo.toml:27`). So **any binary that can call
`serve` has `Value: Send + Sync`** — handler closures and their captured `Value` graph can be shared
across threads. This is the load-bearing design decision (and the reason a JS event loop would be the
*wrong* model — see §4).

### 3b. Two stacking layers of parallelism

`serve_impl_with_factory` (`crates/tish_runtime/src/http.rs:874-1044`):

1. **Prefork — multiple OS processes** (`http.rs:910-933`, `http_prefork.rs`). The default native
   layout: the binary re-execs itself once per core (`std::process::Command`, not `fork(2)`), each
   child bound to the same port via **`SO_REUSEPORT`** (`http.rs:750-778`); the kernel load-balances
   `accept()`. Each child **re-runs the whole program from scratch** and has its **own VM and its own
   `Value` graph** — total isolation, like nginx/gunicorn/unicorn/puma/php-fpm.
2. **OS threads within a process** (`http.rs:1003-1018`). When prefork is disabled, N accept threads
   each run the VM inline, **sharing one handler `Value`** via `Arc` — memory-safe precisely because
   `send-values` made `Value: Send + Sync` (`http.rs:880,1007-1012`).

So sharing is layer-dependent: **across processes = isolated; across threads = genuinely shared (via
`Arc<Mutex>`)**. `serve(..., { onWorker })` opts each thread into building its own state instead.

### 3c. How one request runs

tiny_http backend (default): the request handler runs **synchronously, inline, on the OS thread that
accepted the connection** — no cross-thread queue (`worker_loop_direct`, `http.rs:1138-1182`):
`let response_value = handler(&[req_value]);` is the bytecode VM executing the user handler to
completion on that thread. N threads/processes ⇒ N requests truly in parallel. (The opt-in hyper
backend funnels to a single VM dispatcher thread reached over an mpsc channel,
`http_hyper.rs:187-198`.)

### 3d. Async I/O inside a handler

Each OS thread owns a **multi-threaded tokio runtime** (`new_multi_thread().worker_threads(4)`,
`http.rs:42-48`):
- `fetch(url)` **spawns the request onto tokio immediately** and returns a `Promise` holding a
  `oneshot::Receiver` (`http_fetch.rs:399-424`) — it's in flight before you await.
- `await` then **blocks that worker thread** until the oneshot resolves (`block_on_http` spawns a
  scoped thread doing `rt.block_on`, `http.rs:52-65`) — it does not yield to a loop.
- `fetchAll(urls)` is the true fan-out: one `rt.spawn` doing `futures::future::join_all`
  (`http_fetch.rs:476-484`) — all requests concurrent, joined once.
- `Promise.all(arr)` blocks on each element **sequentially** (`promise.rs:118`); parallelism still
  happens only because each `fetch` was already spawned at *creation* time. For real fan-out, use
  `fetchAll`.

### 3e. The model in one sentence

Concurrency = **(prefork processes ⊕ accept threads) for request-level parallelism** × **per-thread
tokio for I/O-level concurrency inside a handler**, with the VM running each handler straight-line and
blocking on `await`. No reactor multiplexes handlers; the kernel and the thread/process pool do.

---

## 4. Why tish does NOT adopt a JS event loop (design rationale)

A JS-style single-threaded event loop with a microtask queue is **fundamentally at odds** with the
model in §3:

- The event loop assumes **one thread** owns all state and never blocks; concurrency is cooperative
  (callbacks yield to the loop). tish instead runs handlers **in parallel on many threads/processes**
  and lets `await` **block** the calling thread — which is fine because there are many of them.
- `send-values` (`Arc<Mutex>` Values, `Send + Sync` handlers) exists **specifically** to share state
  across those threads. A single-threaded loop would make that machinery pointless overhead.
- Retrofitting "all microtasks drain before the next macrotask, on one thread" would mean serializing
  what is currently parallel, and re-introducing a cooperative-yield discipline that the blocking-
  `await` + thread-pool design deliberately avoids.

So tish trades JS's **deterministic single-thread ordering** for **real multi-core parallelism**.
The cost is that JS's micro/macro-task **ordering guarantees do not hold** (§2e), and `setTimeout` is
best-effort outside the interpreter (§2d). tish targets the multi-threaded server/script niche, not
Node's event-loop semantics.

---

## 5. Practical implications / gotchas

- **Don't rely on `.then` callbacks running if you never `await` the chain** (VM/native drop them).
  Prefer `await`. The interpreter runs them eagerly/synchronously, which is *also* not JS-faithful.
- **`setTimeout` is reliable only in the interpreter.** On the VM it fires only if already due at exit;
  on rust-native it fires only during blocking I/O. Don't use `setTimeout` for "run later" in compiled
  binaries.
- **`await` blocks a thread.** In an HTTP handler that's fine (there are many threads). In a tight
  single-threaded script, a slow `await` blocks everything — there is no loop to interleave other work.
- **No micro-before-macro ordering.** Code that depends on JS's `Promise.then`-before-`setTimeout`
  ordering will behave differently on every tish backend.
- **`async function` ≠ returns-a-Promise.** It's a plain function here; only `await` (which blocks) and
  explicit `Promise` construction give async-ish behavior.
- **Parallel HTTP is a strength.** `fetchAll` + the prefork/thread model give real multi-core scaling
  that a single-threaded event loop cannot — this is the deliberate trade.

## Key source references

- Promises (VM/native): `crates/tish_runtime/src/promise.rs` (`ThenPromise:78`, `promise_object:162`,
  `await_promise:239`); core trait `crates/tish_core/src/value.rs:109,297`.
- Promises (interp): `crates/tish_eval/src/promise.rs`; `eval.rs:2500-2806` (.then/resolver cascade),
  `:3734` (await), `:2845` (`run_timer_phase`).
- Timers: `crates/tish_runtime/src/timers.rs` (drain `:46`, sort `:68`, register `:107`); interp clone
  `crates/tish_eval/src/timers.rs`; VM drain `vm.rs:1820`; WS opportunistic drain `ws.rs:145`.
- `await`: VM `vm.rs:1736`; native `codegen.rs:3604`; async-as-sync `codegen.rs:2572`,
  `compiler.rs:983`, `eval.rs:522`.
- Multi-thread HTTP: `crates/tish_runtime/src/http.rs` (serve `:874`, reuseport `:750`, per-thread run
  `:1138`, tokio `:42`), `http_prefork.rs`, `http_hyper.rs`, `http_fetch.rs:399-489`; `send-values`
  `crates/tish_core/src/vmref.rs:57,111` + `value.rs:68`.
