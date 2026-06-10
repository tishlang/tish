# HTTP server / TechEmpower status (analysis 2026-06-10)

**Question:** is the multithreaded, non-blocking HTTP server still working as part of the TechEmpower
(TFB) requirements, after the typing / stdlib-types work?

**TL;DR — yes, the server works.** The default `tiny_http` backend serves correctly and concurrently
on both the VM and the native-AOT path (prefork, 14 processes here), the multithreaded handler-
dispatch regression test passes, and none of this session's typing/stdlib changes regressed it. The
**hyper** backend (which the TFB build selects via `--feature http-hyper`) was broken — but by the
`#78` perf-branch merge (2026-06-07), **not** this session; it is **fixed here** with two small
changes. The only remaining blocker for the *DB-backed* TFB endpoints is the **`tish-pg` sibling
crate**, which is stale against the post-`#78` `tish_core` API (separate repo, pre-existing).

## Architecture (what "multithreaded non-blocking" means here)

`serve(port, handler)` / `serve(port, { onWorker })` from `import … from 'http'`.

- **Multithreading = prefork.** N processes (default = CPU count; `TISH_PREFORK_WORKERS` /
  `TISH_HTTP_WORKERS` override) each bind the port with `SO_REUSEPORT`; the kernel load-balances
  connections across them. Keeps the per-process VM single-threaded while using all cores.
- **Two backends:**
  - `tiny_http` (default): each process has accept thread(s); under `send-values` the handler is an
    `Arc<dyn Fn + Send + Sync>` cloned per thread and **called in parallel**. (Without `send-values`,
    an mpsc queue serializes to one VM thread.)
  - `hyper` (opt-in `--feature http-hyper` + `TISH_HTTP_BACKEND=hyper`): per-process single-threaded
    tokio runtime, async non-blocking accept, handler run via the dispatch loop. This is the backend
    the TFB `tish-rust` variant builds.
- **`send-values`** (forced on by the `http` feature) makes `Value` = `Arc<Mutex<…>>` (`Send + Sync`)
  so the handler can cross threads. The TFB perf note (`docs/perf.md`) calls this out as a hard
  dependency for multi-threaded dispatch + tish-pg pipelining.
- macOS caveat (documented): BSD `SO_REUSEPORT` funnels accepts to one worker, so true handler
  *parallelism* can't be observed on Darwin — it's a Linux/deployment property. Correctness still
  holds; the cross-platform proof is the Rust thread test below.

## Verification performed (this analysis)

| Check | Result |
|-------|--------|
| `send-values` path compiles (incl. this session's Date/Set/Map/TypedArrays → `Send + Sync`) | ✅ clean |
| `crates/tish_vm/tests/concurrent_shared_state.rs` (12 threads × 100 calls, ~10 handlers in flight, no deadlock) | ✅ pass |
| `tiny_http`, **VM path** (`tish run`): `/plaintext` + `/json`, 16 concurrent | ✅ 200, prefork 14 procs |
| `tiny_http`, **native AOT** (`tish build --native-backend rust`): `/plaintext` + `/json`, 32 concurrent | ✅ 32/32 200 |
| `scripts/test_http_concurrency.sh` shared-counter regression (PREFORK=0, contended) | ✅ pass, no deadlock |
| `hyper` backend, native AOT (`--feature http-hyper --feature process`), 32 concurrent | ✅ 32/32 200, 0 panics *(after the two fixes below)* |
| Full TFB app `tish build src/main.tish` (DB endpoints) | ❌ blocked — `tish-pg` stale API (below) |

## Fixes applied (hyper backend — both pre-existing `#78` breakage)

`crates/tish_runtime/src/http_hyper.rs` (compiled only under `--feature http-hyper`, so the default
build never surfaced these):

1. **Build error** `E0658 use of unstable fn_traits`: `handler.call(&[req_value])` bound the unstable
   `Fn::call` (tuple arg). `handler` is the generic `F: Fn(&[Value]) -> Value`, so → direct call
   `handler(&[req_value])`. (The `tiny_http` path converts to `NativeFn` first, so its `.call` is the
   stable inherent method — that's why only hyper broke.)
2. **Runtime panic** `header_read_timeout set, but no timer set`: hyper 1.x requires a `Timer` whenever
   a timeout is configured. Added `.timer(hyper_util::rt::TokioTimer::new())` to the `http1::Builder`.

After these, the hyper backend builds and serves with no panics.

## Remaining blocker: `tish-pg` (DB endpoints only, pre-existing)

`/Users/a_/Projects/tish-pg` (a **separate repo**, pulled in via the `cargo:` import for
`@tishlang/pg`) does not compile against the current `tish_core`. It uses the **pre-`#78` `Value`
representation** — 8 errors:
- `Value::String(Arc<str>)` → now `Value::String(ArcStr)` (`arcstr::ArcStr`).
- `Value::Object(VmRef::new(ObjectMap))` → now `VmRef<ObjectData>`; build with `Value::object(map)`.
- `.iter()` / `.get()` on `MutexGuard<ObjectData>` → go through `.strings` (the `PropMap`).

These are the same `PropMap` + `ArcStr` refactor `docs/perf-branch-breaking-changes.md` documents, and
identical in shape to the `Value::object(...)` fix already made inside this repo. They block
`/db`, `/queries`, `/cached-queries`, `/fortunes`, `/updates`; `/plaintext` + `/json` are unaffected.

## Reproduce / verify

```bash
cargo build --release -p tishlang --bin tish
cargo test -p tishlang_vm --features send-values --test concurrent_shared_state   # multithread dispatch
./scripts/test_http_concurrency.sh -n 8                                           # counter regression

# tiny_http (default) native:
target/release/tish build tests/http/server.tish -o /tmp/srv --native-backend rust && PORT=8080 /tmp/srv &
curl localhost:8080/plaintext ; curl localhost:8080/json

# hyper (TFB-configured) native:
target/release/tish build tests/http/server.tish -o /tmp/srvh --native-backend rust \
  --feature http --feature http-hyper --feature process
PORT=8081 TISH_HTTP_BACKEND=hyper /tmp/srvh &
curl localhost:8081/plaintext ; curl localhost:8081/json
```

`tests/http/server.tish` is the DB-free TFB plaintext+json fixture; the full benchmark lives in the
`tish-techempower` repo and needs `tish-pg` updated + a Postgres instance.
