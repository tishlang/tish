# tree-sitter-tish

Tree-sitter grammar for [Tish](https://tishlang.com) (incremental subset). Used by:

- [ast-grep](../../tish-security/ast-grep/) in the sibling **`tish-security`** repo (see `sgconfig.yml`)
- Future OpenGrep / editor integrations (see [tish-security/opengrep/UPSTREAM.md](../../tish-security/opengrep/UPSTREAM.md))

## Build

```bash
npm install
npx tree-sitter generate
```

Commit the generated C sources (`src/parser.c`, etc.) after changing `grammar.js`. **CI only runs `generate` and `git diff`** — no C compiler on the runner — so if you forget to regenerate, the job fails.

**Local** corpus checks (`npx tree-sitter test`, `parse`) compile that C with your system toolchain (gcc/clang). Skip those in CI if you want zero native compilation in pipelines; run them before pushing when you touch the grammar.

## Parse a file

From this directory:

```bash
npx tree-sitter parse path/to/file.tish
```

If the CLI prints *You have not configured any parser directories*, run **`tree-sitter init-config`** once (writes `~/.config/tree-sitter/config.json`), or add this grammar’s directory to `parser-directories` in that file. **`--config-path`** on `parse` / `test` is for an alternate *grammar project root* (a folder containing `tree-sitter.json`), not that global config — CI instead creates `~/.config/tree-sitter/config.json` on the runner.

## Notes

- **Do not** add a extra JSON file named like `tree-sitter-ci-config.json` in this directory. The Tree-sitter CLI scans `tree-sitter*.json` paths and expects each to be a **directory** containing `tree-sitter.json`; a stray **file** with that pattern produces: `Failed to parse ./tree-sitter-ci-config.json/tree-sitter.json -- Not a directory`.
- **Return:** `return` without a value must use `return;` (no implicit semicolon insertion).
- **Coverage:** Grow the grammar with `docs/LANGUAGE.md` and `tests/core/*.tish` in this (`tish`) repository.
