# Archiving `tishlang/tish-pg`

The `tishlang_pg` crate and `@tishlang/pg` npm package now live in **`tishlang/tish`**:

- Rust: `crates/tish_pg/`
- npm: `npm/pg/`

Before archiving the old repo:

1. Push a final **`README.md`** to `tishlang/tish-pg` stating the code moved to `https://github.com/tishlang/tish/tree/main/crates/tish_pg` and npm `@tishlang/pg`.
2. Run: `gh repo archive tishlang/tish-pg --confirm` (requires maintainer token).

No further releases should be tagged on the standalone repository.
