# Code audit: cleanup, optimize, secure (June 2026)

A three-dimension audit of the tish workspace (core + runtime crates), with what was
fixed in this pass and a prioritized roadmap for the rest. Findings cite `file:line`.

## Unifying themes

1. **Interp/core duplication is the #1 structural risk.** `tish_eval` (the tree-walk
   interpreter) carries private re-implementations of semantics that `tish_core` /
   `tish_builtins` already provide for the VM/compiled paths — JSON, object layout,
   equality, `Math.*`, `console`, even a second HTTP server. They drift silently; several
   already had. The durable fix is to make `tish_eval` *consume* the shared crates.
2. **The HTTP server shipped with essentially no DoS limits** on either backend (body,
   headers, timeouts) plus SSRF/CRLF gaps. Production runs the VM/compiled path.
3. **Hot-path allocations** — per-call closures, per-element arg slices, per-object Vecs.

## Fixed in this pass (verified + regression-tested)

### Security
| id | fix | file |
|----|-----|------|
| C1 | `JSON.parse` stack-overflow → SIGABRT remote crash: depth limit (128) | `tish_core/src/json.rs` |
| C2 | `JSON.parse` O(n²) `parse_number` (chars().collect per number): byte-scan, 4500ms→2ms | `tish_core/src/json.rs` |
| M2 | CRLF response-header injection on tiny_http: reject CR/LF/NUL in header key/value | `tish_runtime/src/http.rs` |
| H1 | unbounded request body → OOM: cap read at `TISH_HTTP_MAX_BODY` (16 MiB) both backends | `http.rs`, `http_hyper.rs` |
| H2 | slowloris on hyper: `header_read_timeout(30s)` | `tish_runtime/src/http_hyper.rs` |
| M1 | `fetch` no-timeout + unbounded redirects: cached client w/ request+connect timeouts, `redirect(limited(5))` | `tish_runtime/src/http_fetch.rs` |
| M3 | WebSocket unbounded accepts: max-connections cap (`TISH_WS_MAX_CONNS`, 10000) | `tish_runtime/src/ws.rs` |

### Correctness / convergence
- **#1** `Math.random` used a non-uniform RandomState hash in the interp → now `rand::random::<f64>()`, matching every other backend (`tish_eval/src/natives.rs`).
- **#3** loose `==`/`!=` *errored* in the interp → now strict-eq, matching the VM (`vm.rs` maps Eq/Ne to strict_eq). interp == vm == compiled. Locked by `tests/core/jit_regression.tish`.
- **F1** interpreter scope-vars regression (an IndexMap with SipHash used for *variable lookups*, introduced earlier this session) → scopes use aHash again; object-strings keep the insertion-ordered IndexMap (`tish_eval/src/{value,eval}.rs`).

### Optimization / cleanup
- **F4** `Opcode::NewObject` dropped its throwaway `Vec<(Arc<str>,Value)>` — reads pairs in place off the stack into the PropMap, one `truncate` (`tish_vm/src/vm.rs`). Hot path: every `{...}` / JSON response.
- **#10** removed dead `escape_json_string` (`tish_core/src/json.rs`).

## Deferred / remaining roadmap (prioritized)

### Security
- **H3** header count/size limits — tiny_http reads headers into an unbounded Vec; needs socket wrapping or preferring the hyper backend (hyper enforces by default).
- **M1+** SSRF internal-IP block — deny loopback / link-local / RFC1918 / metadata IPs *after* DNS; needs a policy decision (opt-in allowlist) so it doesn't break legit internal use.
- WS **bounded outbound channel** (drop on slow client; currently `unbounded_channel`), prefork **crashed-worker respawn** (`http_prefork.rs:134`), `String.repeat`/`padStart` unbounded-count caps, redact `user:pass@` from WS URL logs. Run `cargo audit` in CI.

### Correctness / convergence
- **`finally` completion bug (BOTH interp + VM)** — a `throw`/`return`/`break`/`continue` inside `finally` should supersede the try/catch outcome (JS) but is swallowed. The interp-only fix breaks interp==vm parity (the VM has the same bug); the VM fix is a bytecode-compiler-level finally-completion change. Fix both together. (#11 reverted to preserve parity; documented in `eval.rs`.)
- **#2** `str.split(/re/)` / `.replace(/re/)` work in the interp but silently no-op in the VM (`tish_builtins/src/string.rs` ignores RegExp args) — add a regex branch.
- **#4** the interp has its *own* JSON parser (`eval.rs:~3105`) — weaker (rejects `\u`/`\b`/`\f`) **and still carries the C1/C2 DoS**. Route through `tish_core::json` (kills the divergence and the interp DoS).
- **#5** `console.log` styled in VM, plain in interp; **#6** interp has a separate `serve()`; **#7** parseInt/parseFloat/Math duplicated verbatim — all collapse by having `tish_eval` consume `tish_core`/`tish_builtins`.
- `value_call` panics (`value.rs:639`) instead of throwing a catchable TypeError when user code calls a non-function.

### Optimization
- **F2** (HIGH) VM allocates a fresh `Arc<dyn Fn>` closure per `arr.map/push/...` call — fuse `GetMember`+`Call` for known method names, or a `CallMethod` opcode.
- **F10a** array builtins alloc a fresh arg slice per element — reuse one `[Value;3]` buffer (like `sort_with_comparator` already does).
- **F9** interp clones a `PathBuf` per function call (`eval.rs:~2583`) — share `Rc<RefCell<PathBuf>>`.
- **F5** `StoreVar` does contains_key+insert (two locks) per scope — collapse to one `get_mut`.
- **F6** `object_get`/`has` allocate an `Arc<str>` per numeric key — format into a stack buffer, look up by `&str`.
- **F12** inline cache on member access, **F13** packed `NumberArray(Vec<f64>)` — the big structural levers (feed the JIT roadmap, tasks #13/#14).

### Cleanup
- **#8** three dead `pub fn` legacy shims in `http.rs` (`request_to_value`/`value_to_response`/`send_response`); **#9** two dead no-op fns in `infer.rs` (with their now-redundant imports).
