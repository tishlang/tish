# Tish

Minimal, TS/JS-compatible language. Runs via interpreter or compiles to native. See `docs/` for spec and ECMA alignment.

## Build & Run

```bash
cargo build -p tish
cargo run -p tish -- run <file.tish>
```

The binary is `target/debug/tish` (or `target/release/tish` with `--release`). Run it directly to skip cargo overhead:

```bash
./target/release/tish run <file.tish>
```

## Test

Full-stack tests (parse → interpret → compile) for all `tests/mvp/*.tish` files:

```bash
cargo test -p tish
```

- `test_full_stack_parse` – lex + parse each .tish file
- `test_mvp_programs_interpreter` – parse + run via interpreter
- `test_mvp_programs_interpreter_vs_native` – compile to native, run, compare output

Run any MVP file: `cargo run -p tish -- run tests/mvp/<name>.tish`.

Manual verification (show output for each .tish file):

```bash
./scripts/run_tests_manual.sh
```

Add `--native` to also compile and run each file natively.

JavaScript equivalents in `performance/mvp/*.js`. Compare Tish vs JS output and timing:

```bash
./scripts/run_performance_manual.sh
```

Runs each pair, prints both outputs, and reports average execution time over 5 runs.
