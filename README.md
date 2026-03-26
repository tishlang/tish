# Tish

> 🚧 Experimental

A highly experimental and opinionated javascript/typescript-like multi-target native compilable language built for friends and family of the JS community.

See [docs/](docs/) for spec and ECMA alignment; user-facing docs: [tishlang.com/docs](https://tishlang.com/docs).

## Quick Example

```javascript
// hello.tish
let name = "World"
console.log(`Hello, ${name}!`)

fn add(a, b) = a + b
console.log(`1 + 2 = ${add(1, 2)}`)
```

## Two Ways to Execute Tish Programs

Tish supports **two execution modes**: interpret or compile to native.

### 1. RUN (Interpreter)

Execute `.tish` files directly without a build step:

```bash
tish run hello.tish
# Hello, World!
# 1 + 2 = 3
```

Best for: development, scripting, quick iteration.

### 2. BUILD (Compile to Native)

Compile `.tish` files to standalone native executables:

```bash
tish compile hello.tish -o hello
./hello
# Hello, World!
# 1 + 2 = 3
```

Best for: distribution, performance, deploying without Tish installed. To deploy apps to the Zectre Platform, see the [Deploy guide](https://tishlang.github.io/tish-docs/deploy/overview/) in the documentation.

The compiled binary is **fully standalone** — no Tish or Rust runtime needed to run it.

### Native backend options

| Backend | Flag | Use when |
|---------|------|----------|
| **rust** | `--native-backend rust` (default) | Full Rust ecosystem; supports native imports (`tish:*`, `@scope/pkg`) |
| **cranelift** | `--native-backend cranelift` | Pure Tish only; faster build, no cargo; errors if native imports present |

```bash
tish compile hello.tish -o hello                    # default: rust backend
tish compile hello.tish -o hello --native-backend cranelift   # cranelift (pure Tish only)
```

### WebAssembly (browser)

Compile to real `.wasm` for the browser:

```bash
tish compile hello.tish -o app --target wasm
# Produces: app_bg.wasm, app.js, app.html
```

**Requirements**: `rustup target add wasm32-unknown-unknown`, `cargo install wasm-bindgen-cli`

Open `app.html` via a local server (CORS): `python3 -m http.server` then visit the URL.

For JavaScript transpilation (no WASM), use `--target js` instead.

### WebAssembly (Wasmtime/WASI)

Compile to a single `.wasm` for [Wasmtime](https://wasmtime.dev) or any WASI runtime:

```bash
tish compile hello.tish -o app --target wasi
wasmtime app.wasm
# Hello, World!
```

**Requirements**: `rustup target add wasm32-wasip1`, [install Wasmtime](https://wasmtime.dev/)

## Installing Tish

### Homebrew (macOS & Linux)

```bash
brew tap tishlang/tish https://github.com/tishlang/tish
brew install tish
```

### Via npx (no install)

```bash
npx @tishlang/tish run hello.tish
npx @tishlang/tish compile hello.tish -o hello
```

Or create a new project:

```bash
npx @tishlang/create-tish-app my-app
cd my-app && npx @tishlang/tish run src/main.tish
```

### From source

```bash
cargo build --release -p tishlang
```

The binary is `target/release/tish`. Add it to your PATH or run directly.

**Note**: Compiling to native (`tish compile`) requires `rustc` and must be run from the workspace root (needs access to `crates/tish_runtime`).

## Developer tooling

Editor tooling is **separate from the compiler** (`tish` = run / repl / compile / dump-ast only).

| Tool | Purpose |
|------|---------|
| **`tish`** | Run, REPL, compile, `dump-ast` — the language implementation. |
| **`tish-fmt`** | Formatter (`cargo build --release -p tish_fmt` → `tish-fmt`). |
| **`tish-lint`** | Linter (`cargo build --release -p tish_lint` → `tish-lint`). |
| **`tish-lsp`** | Language server — links `tish_fmt` / `tish_lint` as libraries for editor integration. |
| **VS Code extension** | [tish-vscode](https://github.com/tishlang/tish-vscode) — grammar, snippets, LSP client, tasks. |

User-facing docs: [Editor & IDE](https://tishlang.github.io/tish-docs/getting-started/editor/), [Language server](https://tishlang.github.io/tish-docs/reference/language-server/), [Formatting](https://tishlang.github.io/tish-docs/reference/formatting/), [Linting](https://tishlang.github.io/tish-docs/reference/linting/). Contributor notes: [docs/tooling.md](docs/tooling.md).

## Using just (Recommended)

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

```bash
cargo test -p tishlang
```

Tests:
- `test_full_stack_parse` – lex + parse each .tish file
- `test_mvp_programs_interpreter` – run via interpreter
- `test_mvp_programs_native` – compile to native, run, compare stdout to static expected (`*.tish.expected`)

Run any test file: `tish run tests/core/<name>.tish`

## Releasing

**→ [How to release (step-by-step)](docs/RELEASE.md)**

Releases are **GitHub-led** and do not modify `main`. The main CI does not push to `main`; it creates a **release branch** and a **GitHub prerelease**.

1. **On push to `main`** (with [conventional commits](#conventional-commits) that trigger a release): CI runs semantic-release in dry-run to get the next version, creates/updates a branch `release/vX.Y.Z`, and creates a **prerelease** on GitHub via the API (with the platform zip attached). There is no version bump or tag on `main`.
2. **When you’re ready**: In GitHub, open the prerelease, attach any extra artifacts if needed, then use **Set as latest release** (uncheck “Set as a pre-release”). That promotes the prerelease to a full release.
3. **Publish workflows** (run when a full release is published or edited to no longer be a prerelease):
   - **NPM**: Publishes `@tishlang/tish` and `@tishlang/create-tish-app` to npm.
   - **Crates.io**: Publishes all `tishlang_*` crates to [crates.io](https://crates.io/crates/tishlang).
   - **Homebrew**: Updates `Formula/tish.rb` in this repo (tap = this repo).

This gives time for the pipeline (or you) to attach the right binaries; publishing only occurs when the release is promoted.

### Required secrets

| Secret | Used by | Purpose |
|--------|---------|---------|
| `NPM_TOKEN` | NPM release | Publish to npm |
| `CARGO_REGISTRY_TOKEN` | Crates.io release | Publish crates |
No extra secrets for Homebrew — it pushes `Formula/tish.rb` to this repo using `GITHUB_TOKEN`.

### Conventional commits

Use these commit message formats so semantic-release can determine the next version:

| Type     | Example                    | Release impact |
|----------|----------------------------|----------------|
| `feat:`  | `feat: add optional chaining` | **Minor** (1.0.0 → 1.1.0) |
| `fix:`   | `fix: correct loop bound`  | **Patch** (1.0.0 → 1.0.1) |
| `perf:`  | `perf: faster parser`      | **Patch** (1.0.0 → 1.0.1) |
| `docs:`  | `docs: update README`      | No release by default |
| `chore:` | `chore: bump deps`        | No release by default |
| Breaking | `feat!: change API` or body `BREAKING CHANGE:` | **Major** (1.0.0 → 2.0.0) |

**Format:** `<type>(<scope>): <description>`, e.g. `fix(vm): handle empty array`.

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