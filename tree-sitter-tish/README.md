# tree-sitter-tish

Tree-sitter grammar for [Tish](https://tishlang.com) (incremental subset). Used by:

- [ast-grep](../../tish-security/ast-grep/) in the sibling **`tish-security`** repo (see `sgconfig.yml`)
- Future OpenGrep / editor integrations (see [tish-security/opengrep/UPSTREAM.md](../../tish-security/opengrep/UPSTREAM.md))

## Build

```bash
npm install
npx tree-sitter generate
```

Optional: `npx tree-sitter test` after adding cases under `test/corpus/`.

## Parse a file

From this directory:

```bash
npx tree-sitter parse path/to/file.tish
```

If the CLI prints *You have not configured any parser directories*, either run `tree-sitter init-config` once on your machine or pass **`--config-path tree-sitter-ci-config.json`** (committed minimal config for this grammar). CI uses that flag so agents stay quiet without a global `~/.config/tree-sitter/config.json`.

## Notes

- **Return:** `return` without a value must use `return;` (no implicit semicolon insertion).
- **Coverage:** Grow the grammar with `docs/LANGUAGE.md` and `tests/core/*.tish` in this (`tish`) repository.
