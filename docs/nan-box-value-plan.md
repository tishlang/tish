# NaN-boxed `Value` — foundational plan (the safe, staged path)

> **Validate — do not trust these numbers.** Any benchmarks, standings, ratios, or
> PASS/acceptance claims below are a point-in-time snapshot and drift the moment the code
> changes — they are illustrative, not ground truth. Re-validate before relying on them:
> `scripts/run_perf_gauntlet.sh` (typed-vs-node PASS/FAIL gate), `scripts/perf_record.sh` +
> `scripts/perf_compare.sh` (over-time, noise-floored), `scripts/run_parity_compare.sh`
> (cross-backend). A verdict means the gate passes **now**, never "we hit X once". Absolute ms
> across different machines/days are not comparable — use a same-machine A/B or the noise-floored
> compare.

**Status:** design + first guard landed (2026-06-06). Implementation is a **dedicated multi-session
workstream** — see "Why this is staged" below. This doc is the executable plan so it can be done
*correctly* (the change is `unsafe` and touches ~600 sites; a logic bug is a silent wrong result).

## Goal & the empirical findings that shape it

Shrink the runtime `Value` so every stack slot / array element / frame local / call arg moves less
memory and number checks become a tag test. The figures below are a **historical snapshot (may be
stale) — regenerate** the sizes with a `size_of::<…>()` probe (the size-guard in `tish_core/value.rs`
fails the build if `size_of::<Value>()` drifts from the asserted target) and re-count the call-site
surface with grep before treating any number as current:

| fact | value | implication |
|---|---|---|
| `size_of::<Value>()` **today** | **24 bytes** | not 16 as the original JSC plan assumed |
| why 24 | `String(Arc<str>)`, `Promise/Opaque(Arc<dyn>)`, `Function(Arc<dyn Fn>)` are **fat (16B) pointers** | the discriminant + a 16B payload ⇒ 24 |
| `size_of::<Arc<str>>()` | 16 (fat: ptr+len) | the blocker |
| `size_of::<Arc<String>>()` | **8 (thin)** | the fix for `String` |
| `Value::Array/Object(VmRef<…>)` | thin (8B) already | fine as-is |
| call-site surface | `Value::String` **409**, `Function` 98, `Promise` 67, `Opaque` 29 (~600) | why it's multi-session |

**Payoff (honest, from the profile — snapshot, re-validate):** the object/array gap is
allocation-*count*-bound; a smaller `Value` cuts memory *traffic* (real, every op) but **not allocation
count**. The relative ranking below — that this ranks **below packed f64 arrays** for raw benchmark
movement — is a point-in-time read of the profile and a standing, not a fixed truth; re-establish it
with `scripts/run_perf_gauntlet.sh` (typed-vs-node PASS/FAIL) and the noise-floored
`scripts/perf_record.sh` + `scripts/perf_compare.sh` against the JS controls before re-prioritizing.
The ~33% / ~67% figures are the *arithmetic* memory-traffic reduction at 16B / 8B (24→16→8 of payload
moved per Value op), not a measured benchmark delta — the actual end-to-end win must be measured, not
assumed. Its value is foundational + uniform: less memory traffic on every Value-moving op across all
VM-family backends.

## Why this is staged (and not a big-bang cutover)

The representation can only be swapped once **every** access goes through an abstraction — otherwise the
swap is thousands of simultaneous edits. And NaN-boxing requires every payload to be ≤8B first (you
cannot NaN-box a fat pointer). So the order is forced:

### Stage A — Abstraction layer (behavior-preserving, suite green throughout)
Add to `tish_core/value.rs`, **without changing the representation**:
- Named constructors: `Value::number(f64)`, `Value::boolean(bool)`, `Value::string(impl Into<…>)`, … —
  every `Value::Number(n)` *construction* site migrates to these.
- `Value::unpack(&self) -> ValueRef<'_>` — a borrowed view enum mirroring today's variants, so every
  `match v { Value::Number(n) => … }` becomes `match v.unpack() { ValueRef::Number(n) => … }`
  (mechanical, compiler-checked).
- Accessors for hot paths that must stay branch-light: `as_number`, `as_bool`, `as_str`, `as_object`,
  `as_array`, `is_truthy`, `is_null`, `tag`.
Then migrate the ~600 sites crate-by-crate (each migration compiles + passes the full suite; the enum
still exists, so unmigrated sites keep working). **This stage has no perf payoff — it is pure enabling.**

### Stage B — Thin the fat variants  → `Value` 24 → 16  (SAFE, `unsafe`-free, INDEPENDENTLY SHIPPABLE)
Make every payload ≤ 8 bytes. This is a real −33% memory-traffic win on its own and the prerequisite
for Stage C. Per variant:
- `String(Arc<str>)` → `String(Arc<String>)` (8B, thin; double-indirection is acceptable) **or** a custom
  thin `TishStr` (`Arc<StrInner{len,[u8]}>`) to avoid the extra `String` header. 409 sites — the bulk.
- `Function(Arc<dyn Fn…>)` (fat) → `Arc<FnBox>` where `FnBox` wraps the `Box<dyn Fn…>` (thin Arc). 98 sites.
- `Promise(Arc<dyn TishPromise>)`, `Opaque(Arc<dyn TishOpaque>)` → same `Arc<Wrapper>` thinning. 67 + 29.
- Verify `Symbol(Arc<TishSymbol>)` and `RegExp(VmRef<…>)` are already thin (they are: `Arc<Sized>` /
  `VmRef`). No change.
- **Gate (re-runnable, validated each run — not a recorded pass):**
  - *Size criterion:* flip the size-guard in `value.rs` from `== 24` to `== 16` — the compile-time
    `assert!(size_of::<Value>() == 16)` fails the build if it regresses; checked on every build/CI.
  - *Suite criterion:* the full backend suite passes (0 failures) — validated by the test run, not by
    citing a past count.
  - *Cross-backend criterion:* `scripts/run_parity_compare.sh` reports `vm ≡ interp ≡ node` (no
    divergence); validated on every parity run.
  - *Perf criterion:* the −33% memory-traffic claim is not self-evident — confirm no regression with
    `scripts/run_perf_gauntlet.sh` (typed-vs-node PASS/FAIL) plus same-machine
    `scripts/perf_record.sh` + `scripts/perf_compare.sh` before claiming the win.
  - Stage B can MERGE on its own as a shipped win before Stage C exists, once the above gates pass now.

### Stage C — NaN-box swap  → `Value` 16 → 8  (the foundational payoff; `unsafe`; hard cutover)
Replace `enum Value` with `struct Value(u64)`:
- Non-NaN f64 stored inline (the hot path — number ops become "is it a non-NaN bit pattern?").
- Quiet-NaN space encodes the other tags + an 8-byte (now-thin) payload — the SpiderMonkey/JSC scheme.
  Suggested layout: top 13 bits = quiet-NaN signal; 3 tag bits select {Null, Bool, Object, Array,
  String, Symbol, Function, Promise/Opaque}; low 48 bits = the thin pointer (x86-64/AArch64 use 48-bit
  virtual addresses, so pointers fit; assert on init).
- Only the Stage-A abstraction methods (`unpack`/accessors/constructors) change impl — the ~600 call
  sites, already on the abstraction, are untouched.
- **`Drop`/`Clone` are the unsafe crux:** `Value(u64)` has no auto-drop of its payload — `Drop for Value`
  must inspect the tag and `Arc::from_raw`/decrement for pointer payloads; `Clone` must `Arc::increment`.
  Get this exactly right or it's a use-after-free / leak.
- **Gates (blocking, re-runnable — each validated on its run, none recorded as "already passed"):**
  - *Size criterion:* size-guard `assert!(size_of::<Value>() == 8)` fails the build on drift; every build/CI.
  - *Suite criterion:* the full 6-backend suite passes (0 failures); the test run, not a cited count.
  - *Differential criterion:* the JIT differential fuzz harness finds no divergence; plus
    `scripts/run_parity_compare.sh` (cross-backend) clean.
  - *Memory-safety criteria:* a **Miri** run over the unsafe core is clean; **ASAN** on the native
    binary is clean; pointer-fits-in-48-bits assert holds at startup; endianness note (the scheme is
    endian-specific — guard or cfg).
  - *Perf criterion:* the 24→8 payoff is a claim until measured — confirm with
    `scripts/run_perf_gauntlet.sh` (typed-vs-node PASS/FAIL) and same-machine
    `scripts/perf_record.sh` + `scripts/perf_compare.sh`.

## Concrete artifact landed now
`tish_core/value.rs` has a `const _:()=assert!(size_of::<Value>()==24)` size guard + a pointer to this
doc. It documents the baseline and turns each stage's size target into a compile-time check (24→16→8),
so the foundation is testable from the first commit.

## Sequencing vs the rest of the perf work
Stage A + B (the safe, shippable 24→16) is the sensible near-term foundational unit. Stage C (the
`unsafe` 16→8) is the dedicated workstream that should land **after** packed f64 arrays — but the
premise behind that ordering (that packed f64 arrays have the higher benchmark payoff) is a
point-in-time standing, not a settled fact; **re-validate the relative payoff** with
`scripts/run_perf_gauntlet.sh` (typed-vs-node PASS/FAIL) and the noise-floored
`scripts/perf_record.sh` + `scripts/perf_compare.sh` before locking the order — unless the goal is
explicitly the Value foundation first. All three preserve behavior across the 6 backends — validated by
the existing differential harness and `scripts/run_parity_compare.sh`, not asserted.
