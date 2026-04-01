# matmul — matrix multiply benchmark

Three compute targets in Tish, all using native imports.

```
examples/matmul/
  tish-metal/          native Tish module — tiled MSL compute kernel
  tish-mlx/            native Tish module — Apple MLX via mlx-rs
  src/
    main.tish          CPU    — import nothing, just Tish primitives
    matmul_gpu.tish    Metal  — import { matmul_f32, run_f32 } from 'tish:metal'
    matmul_mlx.tish    MLX    — import { matmul_f32 } from 'tish:mlx'
  crates/
    matmul_gpu/        standalone Rust binary (direct Metal, no Tish)
    matmul_mlx/        standalone Rust binary (direct MLX, no Tish)
```

The `tish-metal` and `tish-mlx` directories are **local native Tish modules**.
They live here in the example — not in core Tish — and are resolved by the
compiler via the `package.json` / sibling-directory lookup in the native module system.

---

## Running

### CPU (Tish native compilation — f64 primitives)

```sh
tish compile src/main.tish -o matmul-cpu --native-backend rust
./matmul-cpu
```

### Metal GPU

```sh
# From examples/matmul/
tish compile src/matmul_gpu.tish -o matmul-gpu --native-backend rust
./matmul-gpu
```

### Apple MLX

mlx-rs vendors the MLX C library — no `brew install` needed, only Xcode CLT.
First build takes a few minutes while CMake compiles MLX from source.

```sh
# From examples/matmul/
tish compile src/matmul_mlx.tish -o matmul-mlx --native-backend rust
./matmul-mlx
```

### Standalone Rust binaries (no Tish required)

```sh
cargo build --release -p matmul_gpu && ./target/release/matmul-gpu
cargo build --release -p matmul_mlx && ./target/release/matmul-mlx
```

---

## How the native module system works

`import { matmul_f32 } from 'tish:metal'` triggers the compiler to:

1. Look for `tish-metal/package.json` in sibling directories
2. Read `"tish": { "crate": "tish-metal", "export": "metal_object" }`
3. Add `tish-metal` as a Cargo dependency in the generated binary
4. Call `tish_metal::metal_object()` to get the module's export object

The `metal_object()` function (in `tish-metal/src/lib.rs`) returns a
`Value::Object` containing `matmul_f32`, `run_f32`, and `device_name` as
`Value::Function` entries. No changes to core Tish needed.
