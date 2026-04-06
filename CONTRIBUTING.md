# Tish contribution guide

Thank you for your interest in contributing to Tish. Before you start, please read these guidelines. Tish is licensed under the [Pay It Forward License (PIF)](https://payitforwardlicense.com/).

---

## Set up the environment

Fork this repository to your own GitHub account and then clone it locally.

### Rust toolchain

Install a recent **stable** Rust toolchain via [rustup](https://rustup.rs/). The workspace builds with Cargo from the repository root.

### Optional: `just`

Install [just](https://github.com/casey/just) if you want shorthand recipes for run, compile, and secure-mode workflows. Run `just --list` to see available commands.

---

## Install dependencies

Tish is a **Cargo workspace**. Dependencies are fetched automatically on the first build:

```sh
cargo build -p tishlang
```

---

## Making changes and building

### Check out a branch

Create a dedicated branch for your changes:

```sh
git checkout -b MY_BRANCH_NAME
```

### Build the compiler CLI

From the repository root:

```sh
cargo build --release -p tishlang
```

The binary is `target/release/tish`. Add it to your `PATH`, or invoke it via `cargo run -p tishlang -- …`.

### Run a `.tish` file

```sh
cargo run -p tishlang -- run path/to/file.tish
# With optional capabilities, e.g. HTTP:
cargo run -p tishlang -- run path/to/file.tish --features http
```

### Using `just` (recommended)

```sh
just run run hello.tish              # interpreter, full features
just compile hello.tish hello        # native binary (tish build)
just compile-wasi hello.tish hello   # WASI / Wasmtime
wasmtime hello.wasm
just run-secure run hello.tish       # no network / fs / process
```

See `just --list` for all recipes.

---

## Testing

### Add tests

Add or extend tests for bug fixes and new behavior. Integration-style checks live under `tests/` and in crate-local `tests/` directories; prefer covering parsing, the interpreter, and interpreter-vs-native parity where relevant.

### Run unit and integration tests

```sh
cargo test -p tishlang
```

### CI parity (optional)

CI runs `cargo nextest` on **`tishlang`** and **`tishlang_vm`** with `--features full` (see `.github/workflows/build-npm-binaries.yml` for the exact command):

```sh
cargo nextest run -p tishlang -p tishlang_vm --features full --profile ci
```

Without `nextest`:

```sh
cargo test -p tishlang -p tishlang_vm --features full
```

Notable suites include full-stack parse checks, interpreter runs, and native compile-and-compare tests (for example `*.tish.expected` outputs).

You can also execute individual programs with:

```sh
tish run tests/core/<name>.tish
```

---

## Linting

Keep the Rust codebase formatted and warning-clean:

```sh
cargo fmt
cargo clippy -p tishlang
```

Run `clippy` for any additional crate you change (for example `-p tishlang_vm`).

---

## Documentation

- **In-repo (contributors, spec, design):** [docs/](docs/) — language reference ([LANGUAGE.md](docs/LANGUAGE.md)), ECMA alignment, gap analysis, architecture, tooling notes.
- **User-facing site:** [tishlang.com/docs](https://tishlang.com/docs) — installation, guides, and reference.

When you change user-visible behavior, update the site docs in the **tishlang-web** repository where applicable, and keep [docs/LANGUAGE.md](docs/LANGUAGE.md) in sync for the canonical spec.

---

## Related binaries (`tish` CLI vs tooling)

The main **`tish`** binary handles `run`, `repl`, `build`, and `dump-ast` only. Other tools are separate crates:

| Build | Output | Notes |
|-------|--------|--------|
| `cargo build -p tishlang_fmt` | `tish-fmt` | Library: `crates/tish_fmt` |
| `cargo build -p tishlang_lint` | `tish-lint` | Library: `crates/tish_lint` |
| `cargo build -p tishlang_lsp` | `tish-lsp` | Uses `tish_fmt` and `tish_lint` as dependencies |

See [docs/tooling.md](docs/tooling.md) for more detail.

---

## Feature flags

Tish uses compile-time feature flags to gate platform APIs:

| Flag | Enables |
|------|---------|
| `http` | Network (`fetch`, `fetchAll`, `serve`), Fetch-style promises and streams |
| `fs` | File system (`readFile`, `writeFile`, `mkdir`, …) |
| `process` | Process control (`process.exit`, `process.env`, …) |
| `regex` | `RegExp`, `String.match`, … |
| `full` | All of the above |

Default: **no features** (secure mode). For local development you often want `--features full`.

**Log levels:** set `TISH_LOG_LEVEL` to `debug`, `info`, `log`, `warn`, or `error`.

---

## Submitting changes

### Committing and pull requests

Commit your changes on your fork and [open a pull request](https://docs.github.com/en/pull-requests/collaborating-with-pull-requests/proposing-changes-with-pull-requests/creating-a-pull-request) against the default branch. Describe what you changed and why.

Ensure `cargo test -p tishlang` and `cargo clippy -p tishlang` pass (and clippy for any other crates you touched). For language or runtime changes, add or update tests under `tests/` or the relevant crate’s `tests/` directory.

### Format of commit messages and PR titles

Use [Conventional Commits](https://www.conventionalcommits.org/) so release automation can infer versions. Example:

```
feat(vm): add fast path for empty arrays
^    ^    ^
|    |    |__ Subject
|    |_______ Scope (optional)
|____________ Type
```

| Type | Example | Release impact |
|------|---------|----------------|
| `feat:` | `feat: add optional chaining` | **Minor** (1.0.0 → 1.1.0) |
| `fix:` | `fix: correct loop bound` | **Patch** (1.0.0 → 1.0.1) |
| `perf:` | `perf: faster parser` | **Patch** (1.0.0 → 1.0.1) |
| `docs:` | `docs: update README` | No release by default |
| `chore:` | `chore: bump deps` | No release by default |
| Breaking | `feat!: change API` or body `BREAKING CHANGE:` | **Major** (1.0.0 → 2.0.0) |

### Questions and discussion

Open an issue for bugs, ideas, or questions. Documentation improvements, tutorials, and feedback are welcome contributions too.

---

## Releasing

Maintainers: follow the step-by-step guide in **[docs/RELEASE.md](docs/RELEASE.md)**.

Releases are **GitHub-led** and do not bump version on `main` directly. On qualifying pushes to `main`, CI prepares a **`release/vX.Y.Z`** branch and a **GitHub prerelease**; when that prerelease is promoted to **latest**, publish workflows ship **npm** packages, **crates.io** crates, and update the **Homebrew** formula in this repository.

### Required secrets

| Secret | Used by | Purpose |
|--------|---------|---------|
| `NPM_TOKEN` | NPM release workflow | Publish to npm |
| `CARGO_REGISTRY_TOKEN` | Crates.io release workflow | Publish Rust crates |

Homebrew updates use `GITHUB_TOKEN` to push `Formula/tish.rb` in this repo; no extra secret is required for that step.
