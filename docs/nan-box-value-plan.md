# NaN-boxed `Value` — foundational plan (the safe, staged path)

**Status:** design + first guard landed (2026-06-06). Implementation is a **dedicated multi-session
workstream** — see "Why this is staged" below. This doc is the executable plan so it can be done
*correctly* (the change is `unsafe` and touches ~600 sites; a logic bug is a silent wrong result).

## Goal & the empirical findings that shape it

Shrink the runtime `Value` so every stack slot / array element / frame local / call arg moves less
memory and number checks become a tag test. Measured facts (2026-06-06, `size_of` probe + grep):

| fact | value | implication |
|---|---|---|
| `size_of::<Value>()` **today** | **24 bytes** | not 16 as the original JSC plan assumed |
| why 24 | `String(Arc<str>)`, `Promise/Opaque(Arc<dyn>)`, `Function(Arc<dyn Fn>)` are **fat (16B) pointers** | the discriminant + a 16B payload ⇒ 24 |
| `size_of::<Arc<str>>()` | 16 (fat: ptr+len) | the blocker |
| `size_of::<Arc<String>>()` | **8 (thin)** | the fix for `String` |
| `Value::Array/Object(VmRef<…>)` | thin (8B) already | fine as-is |
| call-site surface | `Value::String` **409**, `Function` 98, `Promise` 67, `Opaque` 29 (~600) | why it's multi-session |

**Payoff (honest, from the profile):** the object/array gap is allocation-*count*-bound; a smaller
`Value` cuts memory *traffic* (real, every op) but **not allocation count**, so this ranks **below
packed f64 arrays** for raw benchmark movement. Its value is foundational + uniform: ~33% less memory
traffic at 16B, ~67% at 8B, on every Value-moving op across all VM-family backends.

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
- **Gate:** flip the size-guard in `value.rs` from `== 24` to `== 16`; full suite 14/0; differential
  `vm ≡ interp ≡ node`. Stage B can MERGE on its own as a shipped win before Stage C exists.

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
- **Gates (blocking):** size-guard `== 8`; full 6-backend suite; the JIT differential fuzz harness; a
  **Miri** run over the unsafe core; **ASAN** on the native binary; pointer-fits-in-48-bits assert at
  startup; endianness note (the scheme is endian-specific — guard or cfg).

## Concrete artifact landed now
`tish_core/value.rs` has a `const _:()=assert!(size_of::<Value>()==24)` size guard + a pointer to this
doc. It documents the baseline and turns each stage's size target into a compile-time check (24→16→8),
so the foundation is testable from the first commit.

## Sequencing vs the rest of the perf work
Stage A + B (the safe, shippable 24→16) is the sensible near-term foundational unit. Stage C (the
`unsafe` 16→8) is the dedicated workstream that should land **after** packed f64 arrays (higher
benchmark payoff) unless the goal is explicitly the Value foundation first. All three preserve behavior
across the 6 backends — validated by the existing differential harness, not asserted.
