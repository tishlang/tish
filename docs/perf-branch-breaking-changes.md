# `feature/perf` — branch overview & breaking-change audit

Scope: **47 commits, 149 files** vs `main` (merge-base `0f4a54eb`). A multi-session performance +
JS-conformance effort across the VM, the rust-AOT/cranelift/llvm/wasi backends, the JIT, and the
runtime. This document inventories the **breaking changes in the DEFAULT build** — both the Rust
embedding/extension API and observable tish-language semantics — plus what is additive or flag-gated.

Headline: this branch is **mostly a performance + JS-conformance improvement**, not a redesign. The
"breaking" changes are (a) one hard Rust-API change to how callables/strings are represented, and
(b) several language-semantic changes that are almost all *conformance fixes* (tish behaving more like
JS/Node). Nothing was removed from the tish language surface.

---

## A. Rust embedding / extension API — HARD breaks (default build)

These break downstream Rust crates that embed `tishlang_core`/`tishlang_runtime` or write `cargo:`
native extensions. They are compile-time breaks (the compiler will flag them).

1. **`NativeFn` is no longer a bare `Fn`.**
   - Was: `pub type NativeFn = Arc<dyn Fn(&[Value]) -> Value + Send + Sync>` (`Rc<dyn Fn>` without
     `send-values`).
   - Now: `pub type NativeFn = Arc<dyn Callable>` — a new `pub trait Callable { fn call(&self,
     &[Value]) -> Value; fn as_any(&self) -> &dyn Any; }` (the `as_any` hook lets a VM closure expose
     its compiled chunk).
   - **Migration:** call as `f.call(args)` (not `f(args)`); construct via `native_fn(closure)` or
     `Value::native(closure)` — a blanket `FnCallable<F>` adapter wraps any plain `Fn`.

2. **`Value::String` payload changed: `Arc<str>` → `arcstr::ArcStr`** (a thin 8-byte handle).
   - **Migration:** matching — `s.as_str()` (bare `s.as_ref()` is now ambiguous: `ArcStr` impls both
     `AsRef<str>` and `AsRef<[u8]>`); constructing — `Value::String(x.into())` from `&str`/`String`.

3. **New `Value` variant: `NumberArray(VmRef<Vec<f64>>)`** (packed f64 arrays).
   - Any **exhaustive** `match value { … }` over `Value` in downstream code now fails to compile and
     must add an arm (or `_`). The variant *exists* in every build; it is only *constructed* under
     `TISH_PACKED_ARRAYS=1` (off by default), so a missing arm is a compile break, not a runtime one.

4. **Object storage: `PropMap` is now an `IndexMap`-backed `pub struct`** (was a `pub type PropMap =
   AHashMap<Arc<str>, Value>` alias). `ObjectData` construction helpers changed. Embedders building
   objects via the raw map type are affected; use `Value::object`/`Value::object_from_pairs`.

(`Value::Promise`/`Value::Opaque` remain `Arc<dyn …>` — a 16-byte→thinning experiment was tried and
**reverted** after an interleaved A/B showed it regressed dispatch ~8–10%; see docs/perf.md.)

---

## B. tish LANGUAGE semantics — observable changes on the DEFAULT backend

These can change the output of *existing tish programs*. With one exception (key order) they are
**JS-conformance fixes** — tish now matches Node where it previously didn't.

1. **Division / remainder by zero** → IEEE/JS: `n/0` = `±Infinity`, `0/0` = `NaN`, `n%0` = `NaN`
   (previously **all** `NaN`). Programs that relied on `5/0 === NaN` change. Fixed at all three sites
   (VM `eval_binop`, the `tish_opt` constant-folder, rust-AOT `ops`).
2. **Object key iteration order** → **insertion order** for `Object.keys` / `Object.entries` /
   `for-in` / `JSON.stringify` (previously alphabetical/hash order). Matches JS/Node; the **order** of
   object-iterating output changes (values are identical).
3. **String coercion (ToString)** made JS-conformant for `+`, template literals, `String()`, and
   `Array.join`. String output of some programs changes.
4. **`Array.flat(depth)`** now flattens the requested depth (previously left nested arrays:
   `[1,2,3,4,[5,6]]` → now `1,2,3,4,5,6`).
5. **`number + null` / arithmetic on a non-number** coerces to `NaN` consistently across backends
   (the rust-AOT path previously errored and the codegen swallowed it to `null`).
6. **Promises:** `Promise.race` fixed (resolves to first-to-**settle**, was first-**listed**);
   `Promise.any` and `Promise.allSettled` added; `new Promise(executor)` runs on the VM.
7. **Deeper recursion completes** (the tree-walk interpreter had no stack guard and aborted on deep
   recursion; it now grows the native stack via `stacker`). More capable, not a regression.

No tish syntax, keyword, builtin, or module was **removed or renamed** — the language surface only
grew (RegExp constructor cases, default-param edges, etc., now pass).

---

## C. Default configuration (performance — validated behavior-neutral)

- **ON by default:** slot-based locals, numeric JIT (loops + self-recursion), **array-element JIT**
  (`TISH_JIT_ARRAYS`, *new this branch* — `arr[i]` inside JIT'd loops), `mimalloc` global allocator,
  `parking_lot::Mutex` on the send-values path.
- **OFF (opt-in):** `TISH_PACKED_ARRAYS`, `TISH_FRAME_VM`, and the rust-AOT native-inference toggles
  (`TISH_NATIVE_FN`, `TISH_PARAM_NATIVE`, `TISH_STRUCT_INFER`, `TISH_PARAM_INFER`).
- `send-values` (every `Value` array/object is `Arc<Mutex>`) stays **on** in `full`/`http` builds
  (unchanged) — required for HTTP/WS/PG `Value: Send`.

## D. New capabilities (additive — NOT breaking)

- **`ffi:` portable C-ABI native extensions** (new `tishlang_ffi` crate + `examples/ffi/`) — works on
  all backends, vs the rust-AOT-only `cargo:`.
- Shape registry + per-name inline caches (`tish_core/src/shape.rs`); packed f64 arrays (flag-gated).

## E. Verification status

- Cross-backend integration suite **17/0** (interp · vm · js · rust-AOT · cranelift · wasi).
- rust-AOT compute gauntlet **8/8 beats V8**; vm `fib` beats Node; startup 5ms (fastest engine).
- **Known gaps:** (1) the rust-AOT numeric-local-demotion in `bd8a2901` regresses the
  `typed_assign_conversion` unit test (NOT in CI's `-p tishlang -p tishlang_vm` scope) and over-boxes a
  provably-numeric accumulator. (2) `cargo test --workspace` required migration-fallout fixes (Callable
  `f()`→`f.call()`, ArcStr `.as_str()`) in `tish_ui`/`tish_ffi`/`tish_runtime`/`tish_core` *test* code;
  the shipping library code + CI were unaffected. (3) wasm/wasi has no compute story — the cranelift
  JIT is disabled on wasm targets, so wasi is ~16× Node on the bundle.
