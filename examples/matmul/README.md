# matmul — matrix multiply benchmark

> **Validate — do not trust these numbers.** Any benchmarks, standings, ratios, or
> PASS/acceptance claims below are a point-in-time snapshot and drift the moment the code
> changes — they are illustrative, not ground truth. Re-validate before relying on them:
> `scripts/run_perf_gauntlet.sh` (typed-vs-node PASS/FAIL gate), `scripts/perf_record.sh` +
> `scripts/perf_compare.sh` (over-time, noise-floored), `scripts/run_parity_compare.sh`
> (cross-backend). A verdict means the gate passes **now**, never "we hit X once". Absolute ms
> across different machines/days are not comparable — use a same-machine A/B or the noise-floored
> compare.

Matrix multiply across CPU, Metal GPU, and Apple MLX — with concurrent backend racing
using `Promise.spawn` + `Promise.any`.

```
examples/matmul/
  tish-metal/                   native Tish module — tiled MSL compute kernel
  tish-mlx/                     native Tish module — Apple MLX via mlx-rs
  src/
    main.tish                   CPU only — pure Tish primitives, no imports
    matmul_gpu.tish             Metal only — sequential single-backend benchmark
    matmul_mlx.tish             MLX only  — sequential single-backend benchmark
    matmul_race.tish            MLX vs Metal race — first GPU wins
    matmul_race_settled.tish    MLX + Metal parallel — collect both results & compare
    matmul_fastest.tish         MLX vs Metal vs CPU race — fastest always wins
  crates/
    matmul_gpu/                 standalone Rust binary (direct Metal, no Tish)
    matmul_mlx/                 standalone Rust binary (direct MLX, no Tish)
```

---

## Single-backend benchmarks

### CPU (pure Tish primitives)
```sh
just build-main && ./matmul-main
```

### Metal GPU
```sh
just build-gpu && ./matmul-gpu
```

### Apple MLX
mlx-rs vendors the MLX C library — no `brew install` needed, only Xcode CLT.
First build takes a few minutes while CMake compiles MLX from source.
```sh
just build-mlx && ./matmul-mlx
```

---

## Concurrent backend racing (`Promise.spawn`)

These examples use `Promise.spawn` to launch multiple backends on OS threads
simultaneously, then `Promise.any` / `Promise.allSettled` to collect results.

### `matmul_race.tish` — MLX vs Metal, take the winner

```tish
let winner = await Promise.any([
    Promise.spawn(() => mlx_matmul(a, b, N, N, N)),
    Promise.spawn(() => metal_matmul(a, b, N, N, N)),
])
```

Both GPU backends launch at the same time. The one that returns first wins;
the other keeps running but its result is discarded. Output: winner backend
name, its latency, and wall-clock time.

```sh
just race
```

Illustrative output (snapshot — which backend wins and the ms are machine/day-dependent
and drift; regenerate on your own machine with `just race`, and use
`scripts/perf_record.sh` + `scripts/perf_compare.sh` for a noise-floored comparison):
```
256x256  winner=mlx   backend_ms=4   wall_ms=5   check=63.51...
512x512  winner=metal backend_ms=5   wall_ms=6   check=254.0...
...
```

### `matmul_race_settled.tish` — both backends, compare results

```tish
let results = await Promise.allSettled([
    Promise.spawn(() => { ... return { ms, c } }),   // MLX
    Promise.spawn(() => { ... return { ms, c } }),   // Metal
])
let mlx   = results[0].value   // always index 0 regardless of finish order
let metal = results[1].value
```

`allSettled` waits for both and returns results in the original input order,
regardless of which finished first. Use this for profiling or verifying that
both backends agree on the output (same checksum).

```sh
just race-settled
```

Illustrative output (snapshot — the ms columns are machine/day-dependent and drift;
regenerate with `just race-settled`). The `match` column is a parity criterion, not a
recorded verdict: it must read `yes` (MLX and Metal produce the same checksum) every run,
and cross-backend agreement is gated separately by `scripts/run_parity_compare.sh` — if a
run shows `no`, that is a real failure to investigate, not a stale number:
```
N       mlx_ms  metal_ms  wall_ms  match
256     4ms     5ms       5ms      yes
512     6ms     7ms       7ms      yes
...
```

### `matmul_fastest.tish` — MLX vs Metal vs CPU, always get a result

```tish
let winner = await Promise.any([
    Promise.spawn(() => mlx_matmul(a, b, N, N, N)),
    Promise.spawn(() => metal_matmul(a, b, N, N, N)),
    Promise.spawn(() => cpu_matmul(a, b, N)),        // always available
])
```

`Promise.any` only rejects if ALL promises reject. So if both GPU backends
fail (driver error, kernel timeout, etc.), the CPU result still comes through.
The CPU backend can also win for small N where GPU dispatch overhead dominates —
but which backend wins at a given N is a machine/day-dependent standing, not a fixed
fact: confirm on your own hardware with `just fastest`, and use
`scripts/perf_record.sh` + `scripts/perf_compare.sh` for a noise-floored A/B.

```sh
just fastest
```

---

## How this works: `Promise.spawn`

`Promise.spawn(fn)` runs `fn()` on a background OS thread and returns a
`Promise` that resolves with the return value. Under the `send-values` feature
(the shipped `full` build), the thread is a real OS thread — the GPU kernels
execute concurrently without blocking each other.

`Promise.any([p1, p2, p3])` resolves with the value of the first promise that
**fulfills** (not just settles — a rejection is skipped). If all reject, it
rejects with the array of reasons.

Both are part of the standard `Promise` global — no imports needed.

---

## How the native module system works

`import { matmul } from 'tish:mlx'` triggers the compiler to:

1. Look for `tish-mlx/package.json` in sibling directories
2. Read `"tish": { "crate": "tish-mlx", "export": "mlx_object" }`
3. Add `tish-mlx` as a Cargo dependency in the generated binary
4. Call `tish_mlx::mlx_object()` to get the module's export object at runtime

The export function returns a `Value::Object` with `matmul`, `device_name`,
and `version` as `Value::Function` entries. No changes to core Tish needed.

---

## Standalone Rust binaries (no Tish required)

```sh
cargo build --release -p matmul_gpu && ./target/release/matmul-gpu
cargo build --release -p matmul_mlx && ./target/release/matmul-mlx
```
