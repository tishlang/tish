# Performance guidance from JavaScriptCore / Bun (2026-06-05)

> **Validate — do not trust these numbers.** Any benchmarks, standings, ratios, or
> PASS/acceptance claims below are a point-in-time snapshot and drift the moment the code
> changes — they are illustrative, not ground truth. Re-validate before relying on them:
> `scripts/run_perf_gauntlet.sh` (typed-vs-node PASS/FAIL gate), `scripts/perf_record.sh` +
> `scripts/perf_compare.sh` (over-time, noise-floored), `scripts/run_parity_compare.sh`
> (cross-backend). A verdict means the gate passes **now**, never "we hit X once". Absolute ms
> across different machines/days are not comparable — use a same-machine A/B or the noise-floored
> compare.

We benchmark against Node (V8), but **Bun (JavaScriptCore) is the faster real-world target** and JSC's
architecture is the better source of *techniques*. This maps JSC/V8's core optimizations onto tish's
VM, ranks them by leverage × tractability, and sets the roadmap. Companion to `docs/perf.md` and
`docs/vm-compute-gap-plan.md`.

## The measured gap (compute only, startup excluded — internal `Date.now` Σms)

> **Historical snapshot — may be stale; regenerate before citing.** The ms/ratios below are a
> point-in-time capture, not a live standing, and absolute ms are not comparable across machines.
> Regenerate a same-machine, noise-floored picture with `scripts/perf_record.sh` +
> `scripts/perf_compare.sh` (over-time vs JS controls) and confirm the typed-vs-node verdict with
> `scripts/run_perf_gauntlet.sh` before treating any row as current.

| micro | tish-vm | Node | Bun | tish vs best |
|---|---|---|---|---|
| object_stress | 75ms | 5 | 11 | **15× / 7×** ← worst |
| benchmark_granular | 68ms | 6 | 8 | ~10× |
| new_features_perf | 48ms | 7 | 5 | ~7–10× |
| array_stress | 42ms | 13 | 15 | ~3× |

At the time of this snapshot, after that session's **slot-based locals** + **control-flow JIT**,
pure-numeric *loop functions* matched Node (sumTo 62 vs 63 — snapshot, regenerate with
`scripts/run_perf_gauntlet.sh`). The remaining 3–15× is **everything the JIT doesn't cover: objects,
arrays, top-level loops, and the boxed `Value` itself** — precisely JSC's wheelhouse. Re-measure the
gap with `scripts/perf_record.sh` + `scripts/perf_compare.sh` rather than trusting the multipliers
quoted here and throughout the ranking below.

## tish's architecture vs JSC (the four structural gaps)

| | tish today | JSC | impact |
|---|---|---|---|
| **Value** | 16-byte Rust enum (`tish_core/value.rs:284`); tag + `Arc/Rc`/`f64` payload | 8-byte **NaN-boxed** `JSValue` (doubles inline, int32/ptr/immediate tags) | 2× memory traffic on every stack/array/arg/slot op |
| **Objects** | `PropMap` = `SmallVec<[(Arc<str>,Value);8]>` + `Box<IndexMap>`; lookup = linear/hash per access, no shape | **Structures** (hidden classes) + **inline caches**: `o.x` = structure-check + direct slot load | object_stress 7–15× |
| **Arrays** | `VmRef<Vec<Value>>` — every element boxed | **Array modes / butterfly**: int32/double arrays store raw values | array_stress ~3× |
| **JIT** | one-shot Cranelift, whole numeric leaf/loop fns only, at call time | **tiered** LLInt→Baseline→DFG→FTL with **OSR** + value/array profiling + speculation | top-level loops + polymorphic code never tier up |

Plus: JSC's bytecode is **register-based** (the LLInt) — fewer dispatches than tish's stack VM.

## The techniques, ranked for tish (leverage × tractability)

### 1. Inline caches + Structures for property access  — START HERE
**Why:** object_stress is the single worst micro (7–15×) and `o.x`/`o.x =` is the hottest object op.
**JSC:** each `get_by_id`/`put_by_id` site self-modifies to cache `(structureID, offset)`; on a hit it's
a structure-pointer compare + a direct load. Monomorphic → 1 compare; polymorphic → a small list.
**tish mapping:** (a) give every object a **shape id** = an interned identity for its ordered key-set
(a shape registry: key-sequence → id; `PropMap` insert/delete transitions the id — JSC's *structure
transitions*, themselves cached). (b) Add a per-site **inline cache** to `GetMember`/`SetMember`
(vm.rs:1440): a side table keyed by bytecode offset holding `(cached_shape_id, cached_index)`. On a
shape hit → return `propmap.inline[index]` directly (no key compare, no hash). Miss → slow path +
refill. **Effort: MEDIUM-LARGE.** Carries to cranelift/wasi (embed the VM). The first, canonical JSC win.

### 2. Packed numeric arrays (JSC array modes / butterfly)
**Why:** array_stress ~3×; numeric loops over arrays box every element.
**tish mapping:** a `Value::NumberArray(VmRef<Vec<f64>>)` fast representation; push/index/length/sum
operate on raw f64; auto-box to `Vec<Value>` only at a non-numeric boundary. The **rust backend already
has this** (`tish_compile`); the VM needs its own. **Effort: MEDIUM.** (Was deprioritized as "rust
already wins arrays" — but the VM family is the default and still 3× here, so it's back on.)

### 3. NaN-boxed `Value` (JSC's JSValue) — the foundational broad win
**Why:** halves memory traffic for EVERY value op (stack, array elements, object slots, args) and makes
number checks a tag test, not an enum match. Helps objects, arrays, and compute simultaneously.
**tish mapping:** replace the 16-byte enum with an 8-byte `struct Value(u64)` using NaN-boxing
(doubles inline; pointers/immediates tagged via the NaN space), with accessor methods (`as_number`,
`as_object`, `tag`) replacing today's `match`. **Effort: HUGE** (Value is everywhere; needs `unsafe` +
careful GC/refcount interplay since payloads are `Arc/Rc`). Highest ceiling, highest cost — schedule
after #1/#2 prove the harness, or do it as a dedicated workstream.

### 4. Tiered JIT + OSR (top-level loops, speculation)
**Why:** the JIT only fires on whole numeric functions; top-level hot loops (`jit_probe §06`, 4M iters)
and the interpreted *outer* loop in call-heavy code never go native. JSC OSRs into a running loop.
**tish mapping:** (a) **OSR/region JIT** — compile a hot loop *region* of the main (or any) chunk and
jump into native mid-execution. (b) **profiling-guided speculation** for polymorphic code. **Effort:
LARGE.** Our JIT is already Cranelift (≈ JSC's FTL/B3 tier); the missing piece is OSR + region entry.

### 5. Register-based bytecode (the LLInt lesson)
A register VM cuts the push/pop dispatch count ~30%. **Effort: HUGE** (rewrite bytecode + compiler +
VM). Best bundled with the NaN-box refactor if ever done; lowest priority alone.

## Recommended sequence
**#1 inline caches/structures** (object_stress, canonical JSC win, tractable) → **#2 packed arrays**
(array_stress) → **#4 OSR/region JIT** (top-level loops, compounds the loop-JIT just landed) → **#3
NaN-boxing** as the foundational refactor that lifts everything (dedicated workstream). #5 only if a
ground-up VM rewrite is ever on the table.

Each is a focused multi-step effort, not a tweak — but #1 and #2 are the two that close the biggest
*tracked* gaps (object_stress, array_stress) and are the clearest "be like Bun" wins.
