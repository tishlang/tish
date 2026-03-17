# Hello World

The simplest possible Tish application. Logs output and exits.

## Features Used

None (runs in secure mode).

## What It Does

- Logs a greeting message
- Logs the version
- Exits successfully

## Local Development

Run without installing tish (from this directory; tish repo is `../..`):

```bash
# Run with interpreter
cargo run -p tish --manifest-path ../../Cargo.toml --release -- run src/main.tish

# Compile and run
cargo run -p tish --manifest-path ../../Cargo.toml --release --features full -- compile src/main.tish -o hello
./hello
```

Or with tish installed: `tish run src/main.tish` and `tish compile src/main.tish -o hello`

## Deploy

Deploy with Zectre: `zectre deploy --wait` from this directory. See [Deploy Overview](https://tish-lang.github.io/tish-docs/deploy/overview/) for details.
