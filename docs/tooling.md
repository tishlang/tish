# Tish tooling (contributors)

## Crates

| Crate | Role |
|-------|------|
| `tish` | Binary **`tish`** only — run, repl, compile, dump-ast (no fmt/lint). |
| `tish_parser` / `tish_ast` | Shared by compiler, LSP, fmt, lint |
| `tish_fmt` | Library + binary **`tish-fmt`** |
| `tish_lint` | Library + binary **`tish-lint`** |
| `tish_lsp` | Binary **`tish-lsp`** — tower-lsp; embeds fmt/lint as libraries |

## Running LSP locally

```bash
cargo run -p tishlang_lsp
# Blocks on stdin — editors spawn this; for raw debug, use a client or trace in VS Code (tish.trace.server).
```

## Extending the LSP

1. **`crates/tish_lsp/src/main.rs`** — implement new `LanguageServer` methods; map to `tish_parser` / resolver helpers.
2. **Capabilities** — register in `initialize` (`ServerCapabilities`).
3. **Tests** — integration tests can spawn `tish-lsp` or unit-test helpers extracted from `main.rs` (prefer small modules if the file grows).

## Adding a lint rule

1. Add walk logic in `crates/tish_lint/src/lib.rs`.
2. Push `LintDiagnostic` with a new stable **`code`** (prefix `tish-`).
3. Document in `tish_lint::RULES`, tish-docs, and run `tish-lint` on fixtures.

## Formatter

`tish_fmt` re-emits AST; it does not preserve comments. Major grammar changes require updating the printer in `crates/tish_fmt/src/lib.rs`.

## Release

Ship **`tish`**, **`tish-fmt`**, **`tish-lint`**, and **`tish-lsp`** as separate installable artifacts (same as rustfmt/clippy vs rustc).
