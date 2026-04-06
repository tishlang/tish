# Contributing to Tish

Thanks for your interest in contributing. Tish is licensed under the [Pay It Forward License (PIF)](https://payitforwardlicense.com/).

## Building and testing

- **Build** (from repo root):
  ```bash
  cargo build --release -p tishlang
  ```
  The binary is `target/release/tish`. Add it to your PATH or run directly.

- **Test**:
  ```bash
  cargo test -p tishlang
  ```
  Covers parsing, interpreter, and interpreter-vs-native parity.

- **Run a single file**:
  ```bash
  tish run path/to/file.tish
  # or with features, e.g. http:
  tish run path/to/file.tish --features http
  ```

- **Using just** (recommended): Install [just](https://github.com/casey/just), then run `just --list` for recipes. Examples:
  - `just run run hello.tish` — run via interpreter (full features)
  - `just compile hello.tish hello` — build native binary (`tish build` via just recipe)
  - `just run-secure run file.tish` — run with no network/fs/process

## Code style

- Format: `cargo fmt`
- Lint: `cargo clippy -p tishlang` (and the crate you changed)

## Tooling (separate from `tish` CLI)

- **Compiler CLI**: `tish` — run, repl, build, dump-ast only.
- **Formatter**: `cargo build -p tishlang_fmt` → **`tish-fmt`** binary. Library: `crates/tish_fmt`.
- **Linter**: `cargo build -p tishlang_lint` → **`tish-lint`** binary. Library: `crates/tish_lint`.
- **LSP**: `cargo build -p tishlang_lsp` → **`tish-lsp`** (uses `tish_fmt` + `tish_lint` as deps). See [docs/tooling.md](docs/tooling.md).
- **User docs** in **tish-docs**; update when behavior changes.

## Docs and design

- **In-repo (contributor) docs**: [docs/](docs/) — language spec, ECMA alignment, gap analysis, architecture, tooling.
- **User-facing docs**: [tish-docs](https://tishlang.com/docs) — installation, guides, reference.

## Pull requests

- Open a PR against the default branch. Describe what you changed and why.
- Ensure `cargo test -p tish` and `cargo clippy` pass.
- For language or runtime changes, consider adding or updating tests under `tests/` or `crates/tish/tests/`.

## Questions and discussion

Open an issue for bugs, feature ideas, or questions. We encourage paying it forward: docs, tutorials, and feedback all count.


## Releasing

**→ [How to release (step-by-step)](docs/RELEASE.md)**

Releases are **GitHub-led** and do not modify `main`. The main CI does not push to `main`; it creates a **release branch** and a **GitHub prerelease**.

1. **On push to `main`** (with [conventional commits](#conventional-commits) that trigger a release): CI runs semantic-release in dry-run to get the next version, creates/updates a branch `release/vX.Y.Z`, and creates a **prerelease** on GitHub via the API (with the platform zip attached). There is no version bump or tag on `main`.
2. **When you’re ready**: In GitHub, open the prerelease, attach any extra artifacts if needed, then use **Set as latest release** (uncheck “Set as a pre-release”). That promotes the prerelease to a full release.
3. **Publish workflows** (run when a full release is published or edited to no longer be a prerelease):
   - **NPM**: Publishes `@tishlang/tish`, `@tishlang/create-tish-app`, and unscoped `create-tish-app` (same contents as the scoped scaffold package) to npm.
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