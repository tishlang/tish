# Tish

Minimal, TS/JS-compatible language. Runs via interpreter or compiles to native. See `docs/` for spec and ECMA alignment.

## Build & Run

```bash
cargo build
cargo run -p tish -- run <file.tish>
```

## Test

```bash
cargo test
```

MVP programs live in `tests/mvp/`. Run any with `cargo run -p tish -- run tests/mvp/<name>.tish`.
