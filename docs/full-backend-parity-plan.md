# Plan: make cranelift + wasi full-capability backends (paired via C-ABI FFI; wasi on the WASI ecosystem)

**Status:** plan (2026-06). Goal: cranelift, llvm, and wasi run *everything* the vm/rust backends run,
the only deliberate exception being Rust-crate inclusion (`cargo:`). Native extensions are paired
across all backends through a C-ABI/`extern "C"` mechanism instead of Rust-crate linking, and wasi
gets real access to the WASI ecosystem (fs/clocks/env/stdio + processes) via a wasmtime embedder.

---

## 0. The reframing (the single most important fact)

**cranelift, llvm, and wasi are not three separate compiler backends with their own codegen. They are
three *packagings* of one component — the bytecode VM (`tishlang_vm`).**

| backend | what it actually does | proof |
|---|---|---|
| `--native-backend cranelift` | embeds serialized bytecode as a data symbol, links `tishlang_vm`; cranelift-object is only the **object-file builder** (no CLIF lowering of tish logic) | `tish_cranelift/src/lib.rs:1-5`, `lower.rs:45-59`, `tish_cranelift_runtime/src/lib.rs:29-39` (`Vm::new(); vm.run(chunk)`) |
| `--native-backend llvm` | same, but clang compiles the bytecode-as-`uint8_t[]`; **reuses the cranelift runtime + link path** | `tish_llvm/src/lib.rs:1-5,56-98` |
| `--target wasi` | compiles `tishlang_vm` to `wasm32-wasip1`, embeds bytecode via `include_bytes!` | `tish_wasm/src/lib.rs:271-424`, `tish_wasm_runtime/src/lib.rs:20-26` |

**Consequence: their "limitations" are the bytecode VM's gaps, not backend-specific.** The curated
`CRANELIFT_TEST_FILES` (~20 of ~67) exists because `tish_bytecode` + `tish_vm` lag the tree-walk
interpreter + the rust backend — *not* because cranelift/wasi can't express something. **Fix the VM
once and all four VM-family backends (vm, cranelift, llvm, wasi) become full simultaneously.** This is
a bounded parity effort, NOT the multi-month "lower tish to CLIF/LLVM machine code" project (which
remains explicitly out of scope; the rust backend is already the real native-codegen path).

So the plan is three workstreams, in ROI order:
- **A. VM parity** — closes the pure-tish gap → cranelift/llvm/wasi full for non-native-import programs. *Bounded, low-risk, highest ROI.*
- **B. C-ABI FFI** — a `extern "C"` native-extension mechanism every backend can use, so native code is *paired* (not rust-only). `cargo:` Rust-crate linking stays the one rust-AOT exception.
- **C. WASI ecosystem** — a wasmtime embedder + preview2 so wasi gets fs/env/stdio/clocks (already 90% via std→wasi-libc) **and real processes + sockets/http**.

---

## Workstream A — VM parity (makes cranelift/llvm/wasi full for pure tish)

The interpreter proves the language semantics; the work is bringing `tish_bytecode` (compiler) +
`tish_vm` (runtime) up to it. Every fix here auto-applies to vm, cranelift, llvm, and wasi.

> **STATUS — A1 + A2 LANDED (2026-06-05).** Full **interp↔vm parity across all 66 discovered core
> tests, zero skips** (`VM_PARITY_SKIP` is now empty). Each fix was verified to inherit to **cranelift**
> by building+running the actual backend (all 7 former reds pass). The 6 gaps closed:
> - **number_tofixed** — added `Value::Number` arm to `get_member`; canonical `tishlang_builtins::number::to_fixed`
>   (runtime `number_to_fixed` + the interp route through it — single source of truth).
> - **array_sort_splice** — the fused `ArraySortByProperty` getter (`tish_builtins::array::get_prop_number`)
>   returned `NaN` for string/array `.length` (computed, not a stored key) → every compare collapsed to
>   `Equal` → unsorted. Now computes `.length` for `String`/`Array`, mirroring `get_member`.
> - **default_params** — defaults were silently dropped. Added opcode `ArgMissing(u16)` (#50) + a
>   `emit_param_defaults_prologue` (FunDecl + Arrow, slot + name paths). **Count-based** (default applies
>   only when the positional arg is *missing*, matching the interp; an explicit `null` keeps the `null`).
>   The **native/rust backend** needed the same fix in both codegen param-binding sites.
> - **destructuring** — `compile_destructure` now handles array holes, rest (`...r` via `source.slice(i)`
>   using `GetMember`), and nested array/object patterns (recurse; stack-balanced). Object rest still TODO
>   (no test depends on it).
> - **regex + js_compat_gaps** — RegExp across **VM and native**: `regex` feature now pulls in the runtime;
>   `RegExp` global (`regexp_new`); `Value::RegExp` `get_member` (props + `test`/`exec`); String
>   `match`/`search`/`split`/`replace` route to the runtime's regex-aware fns (same code the rust backend
>   uses). Runtime `get_member` gained the RegExp *properties* (was `test`/`exec` only); `string_split`
>   is now regex-aware; codegen `match`/`search` gated on `has_feature` not `cfg!`. The two `.expected`
>   files were empty placeholders (exposed by file-discovery) — regenerated from the interp.
> - **nested_complex (deep closures)** — `Vm.enclosing` went from a fixed `Option` + `Option` (`enclosing` +
>   `enclosing2`) to a **`Vec<ScopeMap>` full lexical chain** (innermost first). A closure captures its
>   defining frame's scope *plus that frame's own chain*, so functions nested arbitrarily deep resolve every
>   ancestor's locals. Per-iteration `let` (frozen overlay = innermost entry) and captured-var mutation are
>   preserved. `LoadVar`/`StoreVar` walk the chain.
>
> Remaining: **A3** (extend cranelift/wasi from the curated allowlist to `discover_core_tests()`).

**A1 — bytecode compiler gaps (`tish_bytecode/src/compiler.rs`)**
- **Destructuring beyond a bare ident** (`compiler.rs:1227-1281`): array holes, rest `...`, nested
  patterns, renamed object keys all error `"Complex/Nested destructuring not yet supported"`. Implement
  them (the interpreter + rust codegen already do — port the shape).
- **Default parameters**: defaults aren't applied on the VM (`default_params.tish` → `greet()` wrong).
  Fix call-frame setup in the compiler/VM.
- **Deeply-nested closures** (the `nested_complex.tish` bug found by the cross-backend diff): the VM's
  `enclosing` is a *single* level, so a closure nested >1 deep loses grand-parent captures (`level4`
  can't see `level1`'s `a` → `null`). Needs a real **multi-level capture chain** (generalize the
  `enclosing`/`enclosing2` slots added for per-iteration `let` into a `Vec<ScopeMap>` walked by
  `LoadVar`/`StoreVar`). This also retires `VM_PARITY_SKIP` in the test harness.

**A2 — VM runtime gaps (`tish_vm/src/vm.rs`)**
- **Number-primitive methods**: `get_member` (`vm.rs:2105-2192`) has no `Value::Number` arm, so
  `(n).toFixed(2)` / `toString` / `toPrecision` error. Add the arm (impls already exist in
  `tish_core`/`tish_builtins`/`tish_runtime` — reuse, don't re-derive).
- **`RegExp` global**: not registered even with the `regex` feature (`regex.tish` → "Undefined variable
  RegExp"). Register it in `init_globals` under `#[cfg(feature="regex")]`.
- **`.sort()` comparator correctness** (`array_sort_splice.tish` diverges) — fix the VM/`tish_builtins`
  array-method glue.
- Minor: confirm `console.debug`, `Object.keys/values`, `Array.isArray`, `Math.pow/sin/...` are all on
  the VM (the agent found most already are; the gap-analysis doc is partly stale — verify, don't assume).

**A3 — test harness: cranelift/wasi → file discovery.** — *DONE 2026-06-05 (both pass 66/66 via discovery).*

Three fixes made it work: (a) **shared target dir** (`tish_cranelift/link.rs`, `tish_wasm/lib.rs`) so deps
compile once — bounded disk; (b) **unique per-program package name** for cranelift (was the fixed
`tishlang_cranelift_out`, which cross-contaminated builds in the shared target — program B linked
program A's deleted object; wasi already used `tish_wasi_{stem}`); (c) **cache key includes the `tish`
binary mtime** (`compile_cached`) so a VM/codegen edit invalidates stale cranelift/wasi artifacts.
`CRANELIFT_TEST_FILES` removed; `test_mvp_programs_{cranelift,wasi}` use `discover_core_tests()`.

- **The disk/build-cost blocker is FIXED.** Previously each cranelift/wasi case emitted a *full per-program
  Rust crate with its own `target/`* (~2-5 GB with `cranelift_codegen` + the embedded VM); a 66-file sweep
  hit **134 GB / `ENOSPC`**. Fix: build into a **shared** target dir (`tish_cranelift/src/link.rs` →
  `run_cargo_build(.., Some(temp/tishlang_cranelift_target), ..)`; `tish_wasm/src/lib.rs` → shared
  `tishlang_wasi_target`). The heavy deps now compile ONCE and are reused; only each program's tiny main +
  object/chunk rebuilds. Measured: full 66-file cranelift sweep = **610 MB** target (was 134 GB) and a 2nd
  build is **~1 s** (deps cached). cargo's target lock + the nested-cargo mutex serialize concurrent builds
  safely. (llvm uses clang, not a per-program cargo target — no change needed.)
- **Correctness: cranelift passes all 66 discovered tests (66/66)** — full VM-parity inheritance confirmed by
  the disk-safe sweep. So cranelift's skip set is empty; the test moves from the curated `CRANELIFT_TEST_FILES`
  to `discover_core_tests()`. (wasi: same sweep determines its skip set — e.g. regex if the wasi build omits
  the feature.)
- Timing/perf/probe files (`*_stress`, `*_perf`, `jit_probe`, `benchmark_granular`) stay excluded
  always (nondeterministic ms output), independent of VM correctness.

**Scope:** a handful of focused, independently-testable fixes. No structural rewrite. This is most of
"make cranelift/wasi not limited."

---

## Workstream B — C-ABI / `extern "C"` FFI (paired native extensions across all backends)

**Today:** native modules cross the boundary as `Arc<dyn Fn(&[Value]) -> Value>` where `Value` is a
non-`#[repr(C)]` Rust enum (`tish_core/src/value.rs:284-300`) — so native code must be the *same* Rust
compilation (the `LANGUAGE.md:25` "conflicting Value types" warning). All native linking is
compile-time; there is **no FFI/`dlopen`** anywhere. cranelift/llvm/wasi hard-reject external native
imports (`tish_native/src/lib.rs:164-168,208-212`; `tish_wasm/src/lib.rs:279-282`). `cargo:` is
rust-AOT-only.

**Target:** a stable C ABI so native extensions are C-ABI artifacts (cdylib / wasm host imports),
loadable by every backend — the "pairing." `cargo:` (Rust-crate compile-time linking) remains the one
rust-only path.

**B1 — define the C ABI (a new `tish_ffi` crate + header)**
- Opaque value handle: `typedef struct TishValue* TishValueRef;` (a boxed index into a VM-side value
  table or a `*mut c_void`) — never the Rust enum by value (it can't cross `extern "C"`).
- Host-provided accessor API (all `extern "C"`): `tish_value_new_number/string/bool/null`,
  `tish_value_array_new/push/get/len`, `tish_value_object_new/set/get/keys`, `tish_value_tag`,
  `tish_value_clone`, `tish_value_drop`. This decouples extensions from `tish_core`'s version/features.
- Native-fn signature: `extern "C" fn(args: *const TishValueRef, argc: usize) -> TishValueRef`.
- Module entry: one `#[no_mangle] extern "C" fn tish_module_register() -> *const TishExportTable`
  returning a name→fn-pointer table.

**B2 — the loader + dispatch shim**
- Native: `libloading::Library` (new dep) loads a cdylib; for each export, store a `Value::native`
  Rust *shim* that marshals `&[Value]`→handles, calls the `extern "C"` fn, unwraps the result handle.
- Reuse the existing registration chokepoints unchanged: interp `Evaluator::with_modules` /
  `virtual_builtins` (`tish_eval/src/eval.rs:290-309`) and VM `register_native_module` /
  `LoadNativeExport` (`tish_vm/src/vm.rs:849-853,1788-1811`). Only the *backing* of the fn changes from
  a direct Rust call to an FFI trampoline.

**B3 — wire across the VM-family backends**
- `tish_cranelift_runtime`'s entry today is just `Vm::new(); vm.run(chunk)` — teach it (and the llvm
  reuse) to discover + `register_native_module` the FFI modules a program imports.
- Introduce an `ffi:` (or `extern:`) import specifier for C-ABI modules — allowed on **all** backends.
  Relax `has_external_native_imports` to permit `ffi:` everywhere while still rejecting `cargo:` on
  non-rust backends. (Document: `cargo:` = rust-AOT Rust crates; `ffi:` = portable C-ABI.)

**B4 — wasm/wasi binding (shares the contract, different mechanism)**
- There is no `dlopen` in wasm. The same handle+accessor contract is satisfied by **wasm host imports**
  (functions the embedder resolves on the wasmtime `Linker` — workstream C) or WASI. So a `ffi:` module
  on wasi resolves to host imports rather than a cdylib; one ABI, two bindings.

**Scope:** significant — new ABI, loader, marshaling, per-backend wiring. Hard parts: Value-as-Rust-enum
forces the opaque-handle + accessor API (marshaling cost the current zero-copy `&[Value]` call avoids),
and the rust backend's inlined native calls (`codegen.rs:1014-1030`) would move to runtime FFI
indirection (a perf trade on that backend — keep `cargo:` inline there).

---

## Workstream C — WASI ecosystem (wasi gets real fs/env/stdio + processes)

**Today (the good news):** outside `tish_vm` there are **zero** `cfg(wasm32/wasi)` branches — fs,
clocks, random, args, env, stdio all flow through plain `std` → wasi-libc → **real WASI preview1
syscalls** (`fd_write`, `clock_time_get`, `random_get`, `args_get`, `environ_get`). They already work —
*if the runtime is given capabilities*. The gaps:
1. The harness invokes the bare `wasmtime` CLI with **no preopens/env** (`integration_test.rs:1038`), so
   fs/env are effectively dead in tests.
2. **Process spawning is a true hole**: `process.exec` → `std::process::Command::new("sh")`
   (`tish_runtime/src/lib.rs:555-569`) is `Unsupported` on wasip1 → silently returns exit 1.
3. **HTTP/WS dropped**: the tokio/reqwest stack doesn't build for wasm, so `http`→`promise` downgrade
   strips `fetch`/`serve`/`ws` (`tish_wasm/src/lib.rs:33-47`).
4. No cranelift-JIT on wasm (interpreter-only) — a perf ceiling, not a correctness gap.

**C1 — ship a wasmtime *embedder* (the foundation)**
- tish has no embedder today (it shells out to the `wasmtime` binary → no `Linker` to extend). Add
  `wasmtime` + `wasmtime-wasi` as host deps; build a `Linker` + `WasiCtx` with preopens, `--env`, args,
  inherited stdio. This is the keystone — it unlocks C3, C4, and B4.

**C2 — move to WASI preview2 / component model** (currently preview1 `_start` command modules only;
no preview2/component anywhere in the tree)
- Output preview2 *components* (not just preview1 command modules) so we can import `wasi:http`
  (outbound fetch), `wasi:sockets` (ws), and provide host-import functions for the `ffi:` path (B4).
- `serve` (inbound) maps to the preview2 `wasi:http/incoming-handler` (run via `wasmtime serve`).

**C3 — real process + ecosystem access**
- `process.exec`: register a host-import shim on the embedder's `Linker` (C1), or use preview2
  `wasi:cli` spawn-style imports, or wasi-libc/WALI POSIX `fork/exec` emulation. Pick host-import shim
  first (smallest, works on preview1 too).
- Plumb preopens/`--env`/args from `tish build`/`tish run` config through to the embedder so fs/env are
  actually reachable (today nothing wires them).

**C4 — harness**
- Replace the bare `wasmtime <file>` test invocation with the embedder (or `wasmtime --dir/--env`) so
  fs/env/process tests can run on wasi as discovery un-curates them (ties into A3).

**Scope:** significant — the embedder (C1) is bounded and high-leverage; preview2 + process + http (C2/C3)
is the deep end.

---

## Sequencing (ROI-ordered)

1. **Phase 1 — Workstream A (VM parity).** Bounded, low-risk, highest ROI: closes the pure-tish gap so
   cranelift/llvm/wasi run the full corpus. Un-curate the test list as fixes land. *Start here.*
2. **Phase 2 — Workstream C1 + C4 (wasmtime embedder + harness preopens).** Gives wasi real
   fs/env/args/stdio and the `Linker` foundation everything else builds on.
3. **Phase 3 — Workstream B (C-ABI FFI).** The unified native-extension mechanism; B4 (wasm host
   imports) builds on C1.
4. **Phase 4 — Workstream C2/C3 (preview2, processes, http/sockets).** The deep WASI ecosystem.

## The deliberate exception
`cargo:` (compile-time **Rust-crate** linking) stays rust-AOT-only — cranelift/llvm/wasi cannot link
arbitrary Rust crates. Everything else — the full language (Workstream A) and portable C-ABI native
extensions (`ffi:`, Workstream B) + the WASI ecosystem (Workstream C) — works on every backend.

## Risks / notes
- `Value` is a Rust enum, not `#[repr(C)]` → the FFI must use opaque handles + an accessor API; expect
  marshaling cost vs today's zero-copy `&[Value]` Rust calls.
- The rust backend should keep `cargo:` inlined (perf); `ffi:` is the portable path for the rest.
- The `nested_complex` deep-closure bug (A1) is the VM scope-chain item already flagged in
  `docs/control-flow-audit.md` — fixing it for parity also closes that correctness gap.
- Real CLIF/LLVM machine-code lowering of tish remains **out of scope** — the rust backend is the
  native-speed path; this plan makes the *VM-family* backends full-capability, not faster.

## Key files
- cranelift: `tish_cranelift/src/{lib,lower,link}.rs`, `tish_cranelift_runtime/src/lib.rs`
- llvm: `tish_llvm/src/lib.rs` (reuses cranelift runtime/link)
- wasi: `tish_wasm/src/lib.rs`, `tish_wasm_runtime/src/lib.rs`
- the shared VM (where parity fixes go): `tish_bytecode/src/compiler.rs` (destructuring 1227-1281),
  `tish_vm/src/vm.rs` (`get_member` 2105-2192, `init_globals` 322+, `enclosing` capture)
- native/FFI: `tish_compile/src/resolve.rs` (`cargo:` 322-361), `tish_vm/src/vm.rs:849-853,1788-1811`
  (`register_native_module`/`LoadNativeExport`), `tish_eval/src/eval.rs:290-309` (`with_modules`),
  `tish_core/src/value.rs:60-71,284-300` (`Value`/`NativeFn`), gating `tish_native/src/lib.rs:164-212`
- harness: `tish/tests/integration_test.rs` (discovery + cranelift/wasi runners ~1006-1065)
