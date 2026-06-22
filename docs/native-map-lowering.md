# Native numeric Map lowering (#320) — design + status

**Goal:** lower a `new Map()` whose keys are all **numeric** and values all **numeric** to a native
`HashMap<u64, f64>` (keyed by canonicalized `f64` bits), instead of the boxed
`IndexMap<Key(Value), Value>`. This is the real fix that closes the k_nucleotide gap (honest gauntlet:
17ms vs node 5ms, 3.4×) — name-independent, flag-gated (`TISH_MAP_NATIVE`, off by default).

## Why f64-bits keys are sound (no integer inference needed)

JS `Map` key identity is **SameValueZero**, which for numbers is bit-equality with two
canonicalizations: `+0` and `-0` are the same key, and all `NaN` are the same key. So:

```rust
fn __tish_map_key(x: f64) -> u64 {
    if x == 0.0 { 0 }                    // +0 and -0 → same key
    else if x.is_nan() { 0x7ff8_0000_0000_0000 }  // all NaN → one key
    else { x.to_bits() }
}
```

is a faithful, total mapping of any numeric key to a hash key — **no unsound `as i64` cast**, works
for fractional keys too. (Matches the runtime `Key(Value)` SameValueZero hashing, but unboxed.)

## Representation

```rust
// tiny multiplicative hasher (u64 keys are already bit patterns; avoids SipHash overhead and any
// extra crate dep — generated code only depends on tishlang_runtime).
#[derive(Default)] struct TishU64Hasher(u64);
impl std::hash::Hasher for TishU64Hasher {
    fn finish(&self) -> u64 { self.0 }
    fn write(&mut self, _: &[u8]) { unreachable!() }
    fn write_u64(&mut self, n: u64) { self.0 = n.wrapping_mul(0x9E37_79B9_7F4A_7C15); }
}
type TishNumMap = std::collections::HashMap<u64, f64, std::hash::BuildHasherDefault<TishU64Hasher>>;
```

Local: `let mut m: TishNumMap = TishNumMap::default();`

## Lowering (extends the #181 direct-dispatch sites)

| Tish | native |
|---|---|
| `m.set(k, v)` (statement) | `m.insert(__tish_map_key(k), v)` |
| `m.has(k)` | `m.contains_key(&__tish_map_key(k))` |
| `m.get(k)` | `*m.get(&__tish_map_key(k)).unwrap_or(&f64::NAN)` |
| `m.size` | `(m.len() as f64)` |
| `for (v of m.values())` | `for &v in m.values() { … }` (v: f64) |

**Soundness notes / detection guards (engage only when all hold):**
- Every `.set` value is numeric; every key is numeric.
- `.get` is used in **numeric context** (missing key → `NaN`, which equals `Number(undefined)`); the
  map must NOT be used in a way that distinguishes `undefined` from `NaN` (e.g. `m.get(k) === undefined`).
- `.set` results are discarded (statement position) — no `let x = m.set(...)` chaining for v1.
- `.values()` appears only as the iterable of a `for…of` with a single binding used numerically.
- No `.keys()/.entries()/.delete()/.forEach()/.clear()`, no passing the map to a function, no
  non-numeric keys/values. Any unsupported use ⇒ that map stays on the boxed `tish_map_*` path.

## ⚠️ Blocking finding (2026-06-22): map lowering alone does NOT help k_nucleotide

The typed k_nucleotide hot loop is **fully boxed arithmetic**, and that — not the map — dominates the
17ms:

```rust
let mut seq = Value::Array(VmRef::new(vec![]));        // boxed array
// ... seq populated via boxed array_push(nextBase()) — nextBase is a boxed closure (nextBase_cell)
let mut key = Value::Number(0_f64);
// inner k=8 loop, ~2.4M times total:
key = ops::add(&ops::mul(&key, &Value::Number(4)), &get_index(&seq, &Value::Number(i+j)));
```

So the win needs a **4-link inference chain**, of which the native map is only the LAST link:
1. `nextBase` → native fn returning `f64` (it mutates the `seed` global and returns number literals;
   currently a boxed closure, and `seq.push(nextBase())` calls it in a boxed context).
2. `seq` → `Vec<f64>` (array-element inference: prove `seq` is `number[]` from `nextBase`'s return —
   chicken-and-egg with the boxed push; needs fixpoint inference like #170/#172/#177).
3. `key` → native `f64`/`i64` (follows once `seq[i+j]` is native).
4. map → native `HashMap<u64,f64>` (this doc — sound, ready, but useless until 1–3 land).

**Conclusion:** #320 is blocked on native array/fn-return inference, the same family as #170/#173/#177.
A map-only lowering would leave ~2.4M boxed ops in place and not approach node — so it is NOT built.
This file keeps the SOUND map design (f64-bits keys) for when links 1–3 land.

## Status

- [x] Validated baseline: k_nucleotide key + `seq` + `nextBase` are all BOXED; the boxed inner-loop
      arithmetic (~2.4M ops) dominates, not the map. Map-only lowering won't help → not built (see above).
- [ ] Flag `TISH_MAP_NATIVE` + `native_map_locals` detection (conservative, all-uses-supported).
- [ ] Emit hasher/key helper + `TishNumMap` local.
- [ ] Dispatch has/get/set/size.
- [ ] Native `for…of m.values()`.
- [ ] Validate: renamed + actual k_nucleotide checksum == node, timing, gauntlet.

Tracked by #203 / #320 (builds on #181). Companion: `.cursor/plans/perf_work_audit_*.plan.md`.
Set `TISH_MAP_DEBUG=1` to trace detection.
