# new expression

Demonstrates **`new`** with **`Uint8Array`** and **`AudioContext`**: the same source compiles on every Tish target; only the JavaScript target emits browser-native `new`.

## Features used

None (secure mode).

## What it does

- `new Uint8Array(4)` — on non-JS targets, a host stub (numeric array of zeros, not a real typed array).
- `new Uint8Array(...[4])` — same stub path with a spread argument list.
- `new AudioContext()` — on non-JS targets, a stub context (e.g. `sampleRate`, no real audio).

For **browser-accurate** behavior, compile with **`--target js`** so the output uses real `new Uint8Array` / `new AudioContext`.

## Node.js / Bun: validate emitted `new`

[`src/node-bun-new-patterns.tish`](src/node-bun-new-patterns.tish) is **JS-target only**: it exercises constructors that exist as globals in modern **Node** (18+ for `fetch`/`Headers`) and **Bun**, so you can confirm the transpiler’s `new` output actually runs.

| Pattern | Why it matters |
|--------|----------------|
| `new Date(...)` | Timestamps, logging, TTLs |
| `new Error(...)` / `TypeError` | Errors without helpers |
| `new Map()` / `new Set()` | Caches, deduping, metadata |
| `new URL(abs)` / `new URL(relative, base)` | Paths, `file://`, joining bases (very common in Node tooling) |
| `new ArrayBuffer` / `new Uint8Array` | Buffers before crypto, streams, `fetch` bodies |
| `new TextEncoder()` | String ↔ bytes (`encode` / `decode` family) |
| `new RegExp(pattern, flags)` | When patterns are dynamic |
| `new Uint8Array(...array)` | Spread in `new` (ES2018) |
| `new Headers()` | Undici/fetch stack (Node 18+, Bun) |

After `tish build … --target js`, run **`node dist/node-bun-new.js`** or **`bun dist/node-bun-new.js`**; each line should end with `true`.

```bash
mkdir -p dist
tish build src/node-bun-new-patterns.tish --target js -o dist/node-bun-new.js
node dist/node-bun-new.js
```

Use a **`tish` binary built from this repository** (e.g. `cargo build --release -p tishlang` from `tish/`, then invoke `../../target/release/tish`). If `which tish` points to an older install, JS output can be wrong (for example `let epoch = new;` instead of `new Date(0)`).

With npm scripts (from this directory, **`tish` on `PATH`**): `npm run verify:js:node` or `npm run verify:js:bun`. After upgrading `tish`, remove stale output: `rm -rf dist`.

Other Node/Bun patterns you may add locally (not in the sample file): `new Promise(...)`, `new (require("node:events").EventEmitter)()`, `TextDecoder`, `DataView`, `WeakMap`, `URLSearchParams`, `Blob` — all depend on how much of that surface you want to support in Tish call/member lowering.

## Local development

From this directory (Tish repo root is `../..`):

```bash
# Interpreter
cargo run -p tishlang --manifest-path ../../Cargo.toml --release -- run src/main.tish

# Native binary
cargo run -p tishlang --manifest-path ../../Cargo.toml --release -- build src/main.tish -o new-demo
./new-demo

# JavaScript (real `new` in output)
cargo run -p tishlang --manifest-path ../../Cargo.toml --release -- build src/main.tish --target js -o new-demo.js
```

With the `tish` CLI installed: `tish run src/main.tish`, `tish build src/main.tish -o new-demo`, and `tish build src/main.tish --target js -o new-demo.js`.

## Deploy

Same as other examples; see the [examples README](../README.md). This sample is primarily for learning `new` semantics across targets.
