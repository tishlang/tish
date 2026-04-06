# Tish

An opinionated javascript/typescript-like memory-safe blazingly fast native compilable language.

See repo [docs/](docs/) for spec and ECMA alignment; user-facing docs: [tishlang.com/docs](https://tishlang.com/docs).

## Installation

Install globally
```sh
brew tap tishlang/tish https://github.com/tishlang/tish
brew install tish
```

or locally with NPM
```sh
npm install @tishlang/tish
```

See [tishlang.com/docs/getting-started/installation](https://tishlang.com/docs/getting-started/installation) for other installation options.

## Quick start
```sh
npx @tishlang/create-tish-app my-app
cd my-app
npx @tishlang/tish run src/main.tish
```

## Building and running tish applications

Tish supports **multiple execution modes** with multiple backends:

### RUN (Interpreter)

Execute `.tish` files directly without a build step. Best for: development, scripting, quick iteration. backends: [vm](https://tishlang.com/docs/getting-started/repl), [interp](https://tishlang.com/docs/getting-started/repl) and [repl](https://tishlang.com/docs/getting-started/repl).

```javascript
// hello.tish
fn greeting(name) = `Hello, ${name}!`
console.log(greeting("World"))
```

```bash
tish run hello.tish
# Hello, World!
```

### BUILD (Compile to Native)

Compile `.tish` files to standalone native executables. Best for: distribution, performance, deploying without Tish installed. Backends:  [rust](https://tishlang.com/docs/reference/native-backend), [cranelift](https://tishlang.com/docs/reference/native-backend), [llvm](https://tishlang.com/docs/reference/native-backend), [wasm web](https://tishlang.com/docs/reference/wasm-targets), [wasi (wasmtime)](https://tishlang.com/docs/reference/wasm-targets), 

```bash
tish build hello.tish -o hello
./hello
# Hello, World!
```

The compiled binary is **fully standalone** — no Tish or Rust runtime needed to run it.

See more details for other targets and run methods in the [tishlang.com/docs/getting-started/first-app](https://tishlang.com/docs/getting-started/first-app) doc.



## Developer tooling

Editor tooling is **separate from the compiler** (`tish` = run / repl / build / dump-ast only).

| Tool | Purpose |
|------|---------|
| **`tish`** | Language - `run`, `repl`, `build`, `dump-ast` |
| **`tish-fmt`** | Formatter |
| **`tish-lint`** | Linter |
| **`tish-lsp`** | Language server — links `tish_fmt` / `tish_lint` as libraries for editor integration. |
| **VS Code extension** | [tish-vscode](https://github.com/tishlang/tish-vscode) — grammar, snippets, LSP client, tasks. |

User-facing docs: [Editor & IDE](https://tishlang.github.io/tish-docs/getting-started/editor/), [Language server](https://tishlang.github.io/tish-docs/reference/language-server/), [Formatting](https://tishlang.github.io/tish-docs/reference/formatting/), [Linting](https://tishlang.github.io/tish-docs/reference/linting/).


## Development

### Using just (Recommended)

The project includes a `justfile` for common tasks:

```bash
# Run a tish file (interpreter, all features)
just run run hello.tish

# Compile to native binary
just compile hello.tish hello
./hello

# Compile to WebAssembly (Wasmtime)
just compile-wasi hello.tish hello
wasmtime hello.wasm

# Run in secure mode (no network/fs/process access)
just run-secure run hello.tish
```

See `just --list` for all available recipes.

## Feature Flags

Tish has compile-time feature flags for security:

| Flag | Enables |
|------|---------|
| `http` | Network access (`fetch`, `fetchAll`, `serve`) — Fetch-style Promises + ReadableStream |
| `fs` | File system (`readFile`, `writeFile`, `mkdir`, etc.) |
| `process` | Process control (`process.exit`, `process.env`, etc.) |
| `regex` | Regular expressions (`RegExp`, `String.match`, etc.) |
| `full` | All features |

Default: **no features** (secure mode). Use `--features full` for development.

**Log levels**: Control output with `TISH_LOG_LEVEL=debug|info|log|warn|error`

## Test

CI (`.github/workflows/build-npm-binaries.yml`) runs `cargo nextest` on **`tishlang`** and **`tishlang_vm`** with `--features full` (see workflow for exact command).

```bash
cargo nextest run -p tishlang -p tishlang_vm --features full --profile ci
# or without nextest:
cargo test -p tishlang -p tishlang_vm --features full
```

Tests:
- `test_full_stack_parse` – lex + parse each .tish file
- `test_mvp_programs_interpreter` – run via interpreter
- `test_mvp_programs_native` – compile to native, run, compare stdout to static expected (`*.tish.expected`)

Run any test file: `tish run tests/core/<name>.tish`

## Performance Comparison

JavaScript equivalents in `tests/core/*.js`. Compare Tish vs Node.js/Bun:

```bash
./scripts/run_performance_manual.sh
```

## Features

- **Variables**: `let` (mutable), `const` (immutable)
- **Functions**: `fn name(a, b) { ... }` or `fn name(a) = expr`
- **Async/await**: `await fetch` / `await fetchAll` (native); interpreter: `--backend interp` for top-level `await`
- **Arrow functions**: `x => x * 2`, `(a, b) => a + b`
- **Template literals**: `` `Hello, ${name}!` ``
- **Control flow**: `if/else`, `while`, `for`, `for..of`, `switch`
- **Operators**: `+`, `-`, `*`, `/`, `%`, `**`, `===`, `!==`, `&&`, `||`, `??`, `?.`
- **Data**: Arrays `[]`, Objects `{}`, with mutation support
- **Built-ins**: `console.log`, `Math.*`, `JSON.*`, `Object.keys/values/entries`
- **Array methods**: `map`, `filter`, `reduce`, `find`, `forEach`, `push`, `pop`, etc.
- **String methods**: `slice`, `split`, `trim`, `toUpperCase`, `includes`, etc.

See `docs/plan-gap-analysis.md` for full feature list and JS compatibility.