# matmul — dense matrix multiply benchmark

Three Tish programs that exercise the same workload on different compute targets.

| File | Target | Requires |
|------|--------|----------|
| `src/main.tish`       | CPU — f64 native Rust loop | — |
| `src/matmul_gpu.tish` | Metal GPU — tiled 16×16 MSL compute kernel | `--feature metal` |
| `src/matmul_mlx.tish` | Apple MLX (Metal) — mlx-rs lazy graph | `--feature mlx` |

Both GPU variants compile to a **single native binary** that calls directly into
Metal via Rust FFI — no subprocess, no Python, no WebGPU.

---

## CPU (`main.tish`)

```sh
# Bytecode VM
tish run src/main.tish

# Native binary (f64 hot loop, no Value boxing)
tish compile src/main.tish -o matmul-cpu --native-backend rust
./matmul-cpu

# JavaScript
tish compile src/main.tish -o matmul.js --target js
node matmul.js
```

Sweeps N = 128, 256, 512. The `number` / `number[]` type annotations lower the
hot inner loop to `f64` / `Vec<f64>` in the generated Rust — no `Value` boxing.

---

## Metal GPU (`matmul_gpu.tish`)

Uses `tish:metal` — a native Tish module backed by the [`metal`](https://crates.io/crates/metal)
Rust crate. A 16×16 shared-memory tiled MSL compute kernel runs directly on the
GPU. The timed pass is preceded by a warm-up pass so shader compilation is not
included in the reported time.

**Requirements:** macOS 13+ · any Metal-capable GPU (Apple Silicon recommended)
```sh
xcode-select --install   # Xcode Command Line Tools
```

```sh
# Interpreter
tish run src/matmul_gpu.tish --feature metal

# Native binary (Tish compiled to Rust, Metal kernel inline)
tish compile src/matmul_gpu.tish \
  -o matmul-gpu --native-backend rust --feature metal
./matmul-gpu
```

Sweeps N = 512, 1024, 2048, 4096.

---

## Apple MLX (`matmul_mlx.tish`)

Uses `tish:mlx` — a native Tish module backed by the
[`mlx-rs`](https://crates.io/crates/mlx-rs) Rust crate (oxideai/mlx-rs),
which wraps Apple's MLX C library. MLX uses lazy evaluation: the matmul graph
is built then dispatched to Metal via `eval()`. Unified memory means no
host↔device copy on Apple Silicon.

**Requirements:** Apple Silicon Mac · macOS 14+ · Xcode Command Line Tools

`mlx-rs` (via `mlx-sys`) vendors the MLX C source and builds it from source
via CMake — no `brew install mlx` or Python needed. Cargo handles everything.

```sh
# Interpreter
tish run src/matmul_mlx.tish --feature mlx

# Native binary
tish compile src/matmul_mlx.tish \
  -o matmul-mlx --native-backend rust --feature mlx
./matmul-mlx
```

Sweeps N = 256, 512, 1024, 2048, 4096.

---

## What the numbers mean

| Backend | What runs on GPU? | Value boxing? |
|---------|------------------|---------------|
| CPU (Rust)   | nothing — SIMD via LLVM | none — `f64` / `Vec<f64>` |
| Metal        | full matmul kernel (tiled MSL) | none — native Rust ↔ Metal buffers |
| MLX          | full matmul (MLX lazy graph → Metal) | none — mlx-rs `Array` types |
| Bytecode VM  | nothing | yes — `Value` enum throughout |
| `--target js`| nothing (JIT in V8/JSC) | JS engine handles it |
