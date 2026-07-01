# HTTP server / TechEmpower status (analysis 2026-06-10)

> **Validate — do not trust these numbers.** Any benchmarks, standings, ratios, or
> PASS/acceptance claims below are a point-in-time snapshot and drift the moment the code
> changes — they are illustrative, not ground truth. Re-validate before relying on them:
> `scripts/run_perf_gauntlet.sh` (typed-vs-node PASS/FAIL gate), `scripts/perf_record.sh` +
> `scripts/perf_compare.sh` (over-time, noise-floored), `scripts/run_parity_compare.sh`
> (cross-backend). A verdict means the gate passes **now**, never "we hit X once". Absolute ms
> across different machines/days are not comparable — use a same-machine A/B or the noise-floored
> compare.

## Measured throughput — tish vs Bun vs node (2026-06-30, same-machine single-worker A/B)

`scripts/run_http_perf.sh` + `scripts/perf_http_3way.sh` (oha, 5s, 64 connections, macOS single
worker — the fair local comparison; multi-worker scaling is a Linux `SO_REUSEPORT` property, see the
caveat below). Point-in-time; re-run to confirm.

| workload | tish | Bun 1.2 | node 24 |
|----------|------|---------|---------|
| HTTP `/plaintext` (req/s, higher=better) | 138,263 | **146,887** | 101,658 |
| HTTP `/json` (req/s) | 136,628 | **150,609** | 98,396 |
| `JSON.stringify`, 100-record doc ×50k (ms, lower=better) | 355 | **224** | 409 |

**Reading:** tish beats node comfortably (HTTP ~1.36×, stringify ~1.15×) but **trails Bun** — HTTP by
~6–10%, and `JSON.stringify` by **~1.6×** (Bun's JSC has a hand-optimized serializer). So the TFB
serialize-and-serve path is **not yet "beats-everything" optimized — Bun is the bar to clear.**

**Optimization targets (to beat Bun, in order of gap size):**
1. **`JSON.stringify`** — the biggest gap (1.6×). The per-key escape scan + object iteration are the
   suspects; Bun caches/fast-paths constant keys and ASCII strings. (Partly addressed since: `#357`
   ryu number→string; `#362` ahash shape registry.)
2. **HTTP per-request path — INVESTIGATED 2026-06-30: NOT a tish-code lever.** Symbol/image-resolved
   `sample` of the running server under `oha` load shows the per-request cost is compute in the
   `tish_http_srv` binary — dominated by **tiny_http's HTTP parse/format**, with **`malloc`
   negligible** (not even a top image). So the request-object build (`into_value`: method/url/path/
   query + the headers map + body) is **not** the bottleneck, even though the TFB handler only reads
   `req.path`. A same-machine single-worker A/B (oha, 48 conns) puts tish's default **tiny_http within
   ~3.5% of Bun** — `/json` 138.9k vs Bun 143.5k, `/plaintext` 138.9k vs 144.1k (tighter than the
   ~6–10% first measured; that was variance). The `hyper` backend is **much slower on macOS
   single-worker** (`/json` ~79k) — it is the Linux per-core + io_uring scaling lever (`#323`), not a
   macOS win. **Conclusion:** the residual gap is the HTTP framework (tiny_http vs Bun's Zig server),
   not tish code; there is no tractable per-request tish-code optimization here. The real lever is
   `#323` (io_uring accept/send batching + hyper-per-core), which is Linux-only.

Separately, the one JSON area where even **node** leads tish is large-payload *parse* (json_roundtrip
~1.15× node, parse-bound: ~400K boxed-object allocations — the boxed-object-model deep track, distinct
from the serialize path above).

**Question:** is the multithreaded, non-blocking HTTP server still working as part of the TechEmpower
(TFB) requirements, after the typing / stdlib-types work?

**TL;DR (snapshot — re-validate, do not treat as a standing verdict)** — at the time of this analysis
the server worked and the full TFB suite (incl. all DB endpoints) built and ran. These are
point-in-time results, not a settled "it's fine now" state; they must be re-checked on every change via
the gates listed below (the "Reproduce / verify" commands, plus `scripts/run_http_perf.sh` for the http
perf gate). The default `tiny_http` backend served correctly and concurrently on both the VM and
native-AOT path (prefork, 14 processes here); the multithreaded handler-dispatch regression test passed;
none of this session's typing/stdlib changes regressed it (re-confirm by re-running the gates). Three pre-existing breakages from the `#78` perf-branch
merge (2026-06-07) — **not** this session — blocked the hyper backend and the DB suite; **all three
are fixed here**, and all 7 TFB endpoints (`/plaintext`, `/json`, `/db`, `/queries`,
`/cached-queries`, `/updates`, `/fortunes`) now serve correctly against a real Postgres.

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

## Verification gates (re-run these — the ✅ below is a recorded snapshot, not a standing verdict)

The ✅ marks are what was observed during this analysis. They are **not** proof the gate passes today —
re-run each command and require the criterion to hold **now** before relying on it. The "Reproduce /
verify" section gives the exact commands.

| Gate (criterion that must hold on re-run) | How / when it runs | Snapshot result (may be stale — regenerate) |
|-------|-------|-------|
| `send-values` path compiles clean (incl. Date/Set/Map/TypedArrays → `Send + Sync`): `cargo build … --features send-values` exits 0 | every build / CI | ✅ clean (snapshot) |
| `crates/tish_vm/tests/concurrent_shared_state.rs` passes (12 threads × 100 calls, ~10 handlers in flight, no deadlock): `cargo test … --test concurrent_shared_state` exits 0 | CI test run | ✅ pass (snapshot) |
| `tiny_http`, **VM path** (`tish run`): `/plaintext` + `/json` return 200 under 16 concurrent | manual repro below | ✅ 200, prefork 14 procs (snapshot) |
| `tiny_http`, **native AOT** (`tish build --native-backend rust`): `/plaintext` + `/json`, 32/32 return 200 | manual repro below | ✅ 32/32 200 (snapshot) |
| `scripts/test_http_concurrency.sh` shared-counter regression passes, no deadlock (PREFORK=0, contended): script exits 0 | `./scripts/test_http_concurrency.sh -n 8` | ✅ pass, no deadlock (snapshot) |
| `hyper` backend, native AOT (`--feature http-hyper --feature process`): 32/32 return 200, 0 panics | manual repro below | ✅ 32/32 200, 0 panics *(after the fixes below)* (snapshot) |
| Full TFB app `tish build src/main.tish` (DB endpoints) builds: exits 0 | `tish-techempower` build | ✅ builds *(after the fixes below)* (snapshot) |
| All 7 endpoints vs local Postgres (`/db`,`/queries`,`/cached-queries`,`/updates`,`/fortunes`): correct rows, writes, HTML+XSS-escaped fortunes, no panics | manual repro below | ✅ correct (snapshot) |
| **HTTP throughput is within the http perf gate vs the JS control** | `scripts/run_http_perf.sh`; validated on each run, not a recorded number | not measured in this analysis — run `scripts/run_http_perf.sh` to obtain |

## Fixes applied (all three pre-existing `#78` breakage, none from this session)

1. **`crates/tish_runtime/src/http_hyper.rs` — build error** `E0658 use of unstable fn_traits`:
   `handler.call(&[req_value])` bound the unstable `Fn::call` (tuple arg). `handler` is the generic
   `F: Fn(&[Value]) -> Value`, so → direct call `handler(&[req_value])`. (The `tiny_http` path converts
   to `NativeFn` first, so its `.call` is the stable inherent method — that's why only hyper broke.)
   This file is compiled only under `--feature http-hyper`, so the default build never surfaced it.
2. **`crates/tish_runtime/src/http_hyper.rs` — runtime panic** `header_read_timeout set, but no timer
   set`: hyper 1.x requires a `Timer` whenever a timeout is configured. Added
   `.timer(hyper_util::rt::TokioTimer::new())` to the `http1::Builder`.
3. **`crates/tish_compile/src/resolve.rs:606` — codegen for `cargo:` native-module wrappers**:
   `generate_native_wrapper_rs` emitted `Value::Object(VmRef::new(m))` (raw `ObjectMap`), which stopped
   type-checking after the `ObjectData`/PropMap refactor → `Value::object(m)`. Affects **every** `cargo:`
   import (not just tish-pg); the wrapper for the `tish_pg` module is what main.tish links.

Plus the **`tish-pg` DB driver** (`/Users/a_/Projects/tish-pg`, a separate repo pulled in via the
`cargo:tish_pg` import, pinned by `tish-techempower/package.json`'s
`"tish_pg": { "package": "tish-pg", "path": "../../tish-pg" }`): it was a stale **pre-`#45`** copy using
the pre-`#78` `Value` API (`Arc<str>`→`ArcStr`, raw `ObjectMap`→`Value::object`, `.iter()`/`.get()` →
`.strings`). The **monorepo `crates/tish_pg` is the already-fixed version of the same crate**, so the
sibling's `src/{lib.rs,error.rs}` were synced from it (its standalone `Cargo.toml` kept). It needs
`send-values`, which it gets from feature-unification when the native build enables `--feature http`.

After all four, `tish build src/main.tish --native-backend rust --feature http --feature http-hyper
--feature process` produces a working binary, and every endpoint returns correct data against Postgres.

## Running the DB endpoints

They need a Postgres with the TFB schema (`sql/create-postgres.sql` → `world` (10k rows) + `fortune`
(12 rows)). Verified locally:

```bash
psql -d hello_world -f sql/create-postgres.sql
PORT=8175 DATABASE_URL="postgres://<user>@127.0.0.1:5432/hello_world" /tmp/tfb_full &
curl 'localhost:8175/db'                  # {"id":…,"randomNumber":…}
curl 'localhost:8175/queries?queries=3'   # 3 rows
curl 'localhost:8175/updates?queries=2'   # 2 rows, written back
curl 'localhost:8175/fortunes'            # HTML table, XSS-escaped
```

The benchmark's own `docker/` compose provides `tfb-database`; `package.json start` points
`DATABASE_URL` at it.

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
