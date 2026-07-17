# Packed f64 arrays (`TISH_PACKED_ARRAYS`)

**Status: sound, opt-in, default OFF. Kept off deliberately — it is a performance wash, not a win.**

Packed arrays give `Value` an unboxed numeric-array representation: `Value::NumberArray`, a
contiguous `Vec<f64>` instead of a `Vec<Value>` of individually-boxed numbers. It is gated behind the
`TISH_PACKED_ARRAYS` env var (read once per process via a `OnceLock`, so it is a whole-run mode, not a
per-value decision).

## Representation (as of #520)

```rust
Value::NumberArray(VmRef<NumArrayBacking>)

pub enum NumArrayBacking {
    Packed(Vec<f64>),    // fast path: all-numeric, unboxed
    Boxed(Vec<Value>),   // deopted: behaves exactly like a boxed Array
}
```

An array literal / numeric `map`/`filter` result starts `Packed`. **Any non-numeric store deopts the
backing in place** — `Packed → Boxed` — via `NumArrayBacking::deopt()`. Because the upgrade mutates
the value behind the shared `VmRef`, **every alias observes the transition**:

```tish
let a = [1, 2, 3]   // Packed(Vec<f64>)
let b = a           // shares the VmRef
a[0] = "x"          // deopts in place -> Boxed(Vec<Value>)
b[0]                // "x"  — the alias sees the upgrade
```

A deopted array stays a `NumberArray` *value* (its aliases can't be rewritten to `Array`), but it
behaves identically to a boxed `Array` from that point on. The paths that deopt: `set_index` with a
non-number, `push`/`unshift`/`fill`/`splice` with any non-numeric element, `delete` (stores a real
`Value::Null` hole), and a `length` grow (fills with `Null` once deopted; `NaN` only while packed).

### Why the enum (history)

The original representation was a bare `VmRef<Vec<f64>>` and used **NaN as a hole/sparse marker**;
storing a non-numeric value was either silently lossy or a no-op, because a shared `&Value` mutation
can't rewrite the aliases' scope bindings from `NumberArray` to `Array`. #520 replaced that with the
in-place-upgradable enum, which *is* the mechanism that lets a shared value deopt soundly without
touching any binding. That fixed #506 (splice non-numeric removed values) and the whole #502–#508
non-numeric-mutation class, plus a pre-existing bug where the no-comparator packed `sort()` sorted
numerically instead of lexicographically (JS default is `String(x)` order).

## Soundness gate (#199)

`TISH_PACKED_ARRAYS=1` must be **observationally identical** to flag-off. Enforced by:

- `scripts/packed_arrays_parity_check.sh` — every `tests/core/*` fixture runs flag-off and flag-on on
  interp / vm / native and must match per backend; every `tests/perf/*` GAUNTLET checksum must be
  identical for vm-off, vm-on, and node. Current: **corpus 162/162, perf 30/30, 0 divergences.**
- `.github/workflows/packed-arrays.yml` — runs that sweep on PRs touching the owning crates, plus a
  `nextest` job that runs the full `tishlang` + `tishlang_vm` suites with `TISH_PACKED_ARRAYS=1`. That
  job is the #199 acceptance gate; it went live on `pull_request` once the sweep's `KNOWN_DIVERGENCES`
  list emptied (#520).

## Performance: why the default stays OFF

The flip was measured both ways on purpose-built large fixtures (`tests/packed_perf/`), VM flag-off vs
flag-on vs node — checksums identical, timings a **wash-to-~1%**:

| fixture | workload | vm off | vm on | node |
|---|---|---:|---:|---:|
| `large_numeric` | build 5M, sum, ×2 in place, reduce | ~1332ms | ~1297ms | ~75ms |
| `many_arrays` | 20k arrays × map/filter/reduce | ~657ms | ~654ms | ~28ms |
| `mutation_mix` | 200k push/pop/splice/sort rounds | ~211ms | ~213ms | ~36ms |

The reason there is no win: **the VM JIT bails on `NumberArray`**, so a packed array never reaches the
JIT tier that actually moves numbers — it stays ~17–23× node either way. Packed's fused HOF paths
(map/filter/reduce staying packed) can show ~1.1–1.24× on isolated chains (see `docs/perf.md` PHASE 2),
but that does not generalize to real workloads.

**Decision:** keep `TISH_PACKED_ARRAYS` default **off**. #520 makes flipping it *sound* if a future
JIT path consumes packed arrays and turns the representation into an actual speed win — until then
there is no performance justification to flip, only representation churn. The flip itself is a
one-line change to `packed_arrays_enabled()`'s default.
