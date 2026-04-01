# Matrix multiply (benchmark-shaped)

Dense N×N multiply with a naive triple loop: flat row-major arrays, `Date.now()` timing, and a small checksum printed to stdout. Uses ordinary Tish numbers (IEEE **double**). Typed `f32` buffers and primitive lowering are **goals** (see `docs/LANGUAGE.md` → *Native compile (implementation status)*), not what the compilers emit for this program yet.

**Default `N` is 256** in `src/main.tish` so the VM and interpreter finish in reasonable time. For **1024×1024**, expect very long runs until true AOT / primitive arrays exist; use smaller `N` for day-to-day timing.

## What the numbers mean (why Cranelift can look “worst”)

| Artifact | Engine | Typical story |
|----------|--------|----------------|
| `tish run` | Bytecode **VM** | Baseline VM. |
| `matmul-cl` (`--native-backend cranelift`) | **Same VM**, bytecode **embedded** in the binary | Cranelift is only used to emit an object file holding the chunk; **`tishlang_vm` runs it**. Throughput is VM-class — often **similar to or a bit worse than** `tish run` (startup, layout). |
| `matmul-rust` (`--native-backend rust`) | Rust + **`tishlang_runtime`** (`Value`, `get_index`, …) | Inner loop still goes through the dynamic runtime, but **less dispatch than the VM** — often **faster than `matmul-cl`**, still usually **slower than Node/Bun** on this loop because V8 **JITs** tight `number` math. |
| `matmul.js` + Node/Deno/Bun | Host JS engine | **JIT** can win big on this microbenchmark. |

So: **neither native backend is “pure Rust matmul” today.** The long-term objective is **primitive lowering** (e.g. `f64`/`f32` buffers, inferred or annotated types) and **real AOT** (bytecode → Cranelift IR or typed Rust), not boxed `Value` everywhere — see the language reference.

## Features used

None (secure mode).

## Run from the Tish repo root

Replace `cargo run -p tishlang --release --` with your installed `tish` if you already have it on `PATH`.

### 1. Bytecode VM (default)

```bash
cargo run -p tishlang --release -- run examples/matmul/src/main.tish
```

### 2. Tree-walking interpreter

```bash
cargo run -p tishlang --release -- run examples/matmul/src/main.tish --backend interp
```

### 3. Native binary — Rust backend (default; `tishlang_runtime`)

Requires `rustc` / Cargo. Use when you need **`tish:*` / npm native modules**. Not peak numeric throughput vs V8.

```bash
cargo run -p tishlang --release -- compile examples/matmul/src/main.tish -o examples/matmul/matmul-rust
./examples/matmul/matmul-rust
```

### 4. Native binary — “Cranelift” (embedded bytecode + VM)

No `rustc` for *your* program. Produces a **standalone** binary that still runs **`tishlang_vm`**. Useful for **shipping** a single executable without a `tish` install — **not** for “fastest matmul.”

```bash
cargo run -p tishlang --release -- compile examples/matmul/src/main.tish -o examples/matmul/matmul-cl --native-backend cranelift
./examples/matmul/matmul-cl
```

### 5. JavaScript (`--target js`) — same emitted file with Node, Deno, or Bun

```bash
cargo run -p tishlang --release -- compile examples/matmul/src/main.tish -o examples/matmul/matmul --target js
```

```bash
node examples/matmul/matmul.js
deno run examples/matmul/matmul.js
bun examples/matmul/matmul.js
```

### 6. WASI WebAssembly + Wasmtime

```bash
cd examples/matmul
cargo run -p tishlang --release --manifest-path ../../Cargo.toml -- compile src/main.tish -o matmul --target wasi
wasmtime matmul.wasm
```

### Compare with hand-written Node (same algorithm)

`compare/matmul.mjs` mirrors the loop and default `N`. Keep `N` aligned with `src/main.tish` when timing.

```bash
node examples/matmul/compare/matmul.mjs
```

## Deploy

This example is local benchmarking only. For platform deploy patterns, see the other examples and [Deploy Overview](https://tishlang.github.io/tish-docs/deploy/overview/).
