# Tish tooling (contributors)

## Crates

| Crate | Role |
|-------|------|
| `tish` | Binary **`tish`** only — run, repl, compile, dump-ast (no fmt/lint). |
| `tish_parser` / `tish_ast` | Shared by compiler, LSP, fmt, lint |
| `tish_resolve` | Lexical resolution (LSP positions, go-to-def, completion names) |
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

### LSP positions vs lexer spans

The language server uses **LSP positions**: 0-based lines, **UTF-16 code unit** columns. Lexer / AST `Span` values (see `crates/tish_ast/src/ast.rs`) are **1-based** lines and **Unicode scalar value** (Rust `char`) columns. Conversions for navigation and edits go through **byte offsets** in the UTF-8 source (`tishlang_resolve::span_to_lsp_range_exclusive`, `span_contains_lsp_position`). Diagnostics from `tish_lint` still report lexer line/column and are mapped approximately in the LSP layer.

## tree-sitter (`tree-sitter-tish`)

Incremental grammar for ast-grep and similar tools — **not** the semantic source of truth (see `docs/LANGUAGE.md`). When you change the surface syntax in `LANGUAGE.md` / `tish_lexer`+`tish_parser`, update `tree-sitter-tish/grammar.js` and run **`tree-sitter generate`** in that directory (requires the [tree-sitter CLI](https://tree-sitter.github.io/tree-sitter/creating-parsers#installation)) so the checked-in C parser stays in sync.

## Adding a lint rule

1. Add walk logic in `crates/tish_lint/src/lib.rs`.
2. Push `LintDiagnostic` with a new stable **`code`** (prefix `tish-`).
3. Document in `tish_lint::RULES`, tish-docs, and run `tish-lint` on fixtures.

## Formatter

`tish_fmt` re-emits AST; it does not preserve comments. Major grammar changes require updating the printer in `crates/tish_fmt/src/lib.rs`. The **tish-vscode** extension defaults **`tish.format.enable`** to off so save does not rewrite sources; users can still run **Format Document** when they want normalized output.

## Release

Ship **`tish`**, **`tish-fmt`**, **`tish-lint`**, and **`tish-lsp`** as separate installable artifacts (same as rustfmt/clippy vs rustc).
