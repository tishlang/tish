# Wasmtime Modular Tish Example

Multiple Tish modules that depend on each other via `import`/`export`, each compilable to a separate WASI `.wasm` binary, runnable via [wasmtime](https://wasmtime.dev).

## Prerequisites

- **Tish CLI** with WASI support: build from the tish repo root with `cargo build --release -p tish` (the `tish` crate includes the `wasi` target). If you see "Unknown target: wasi", you're using an older or different tish binary; run `./build.sh` from this directory so it uses the workspace tish via `cargo run -p tish`.
- `rustup target add wasm32-wasip1`
- [wasmtime](https://wasmtime.dev) installed

## Build

From the tish workspace root:

```bash
./examples/wasmtime-modules/build.sh
```

Or from this directory:

```bash
./build.sh
```

Output: `dist/main.wasm`, `dist/math.wasm`, `dist/greet.wasm`

## Run

**Merged program** (main imports math + greet; compiled as one binary):

```bash
wasmtime dist/main.wasm
```

**Standalone modules**:

```bash
wasmtime dist/math.wasm
wasmtime dist/greet.wasm
```

## How It Works

- **Modular sources**: `math.tish` and `greet.tish` export functions; `main.tish` imports them.
- **Separate binaries**: Each module is compiled as its own entry point, producing a distinct `.wasm`.
- **Merge at compile time**: When building `main.wasm`, Tish resolves and inlines imports from `math.tish` and `greet.tish` into a single program. There is no runtime module loading.
- **Tish WASM model**: Each `.wasm` is the Tish VM with embedded bytecode. Tish does not emit user-exported functions for wasmtime's Linker; linking is conceptual (separate build artifacts) rather than runtime (WASM imports between modules).
