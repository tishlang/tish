# tish-lsp

Language Server Protocol implementation for [Tish](https://github.com/tishlang/tish).

## Build

```bash
cargo build --release -p tishlang_lsp
```

Binary: `target/release/tish-lsp` (stdio LSP).

## Features

- Parse diagnostics + lint warnings (via `tish_lint` **library** — use **`tish-lint`** CLI separately in CI)
- Document symbols, completion, formatting (via `tish_fmt` **library** — use **`tish-fmt`** CLI separately in CI)
- Go to definition (same file, relative `./` / `../`, bare `node_modules` packages like Node, and native `tish:` / `@scope/pkg` / `cargo:` → Rust `pub fn` via `syn` + `cargo metadata` where configured)
- Workspace symbol search (`**/*.tish`)

## Client configuration

See the [Tish docs — Language server](https://tishlang.com/docs/reference/language-server/) and [Editor setup](https://tishlang.com/docs/getting-started/editor/).

## Developing

See the repo [`docs/tooling.md`](../../docs/tooling.md).
