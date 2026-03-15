# Contributing to Tish

Thanks for your interest in contributing. Tish is licensed under the [Pay It Forward License (PIF)](https://payitforwardlicense.com/).

## Building and testing

- **Build** (from repo root):
  ```bash
  cargo build --release -p tish
  ```
  The binary is `target/release/tish`. Add it to your PATH or run directly.

- **Test**:
  ```bash
  cargo test -p tish
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
  - `just compile hello.tish hello` — compile to native binary
  - `just run-secure run file.tish` — run with no network/fs/process

## Code style

- Format: `cargo fmt`
- Lint: `cargo clippy -p tish` (and the crate you changed)

## Docs and design

- **In-repo (contributor) docs**: [docs/](docs/) — language spec, ECMA alignment, gap analysis, architecture.
- **User-facing docs**: [tish-docs](https://github.com/tish-lang/tish-docs) — installation, guides, reference.

## Pull requests

- Open a PR against the default branch. Describe what you changed and why.
- Ensure `cargo test -p tish` and `cargo clippy` pass.
- For language or runtime changes, consider adding or updating tests under `tests/` or `crates/tish/tests/`.

## Questions and discussion

Open an issue for bugs, feature ideas, or questions. We encourage paying it forward: docs, tutorials, and feedback all count.
