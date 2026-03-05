# Async/Await Example

Demonstrates **non-blocking** async/await with `fetchAllAsync` for parallel HTTP fetches.

## Features Used

- `http` - Enables `fetchAsync`, `fetchAllAsync`, and `await`

## Local Development

Build tish with the http feature first, then run or compile:

```bash
# Run with interpreter
cargo run -p tish --features http -- run examples/async-await/src/main.tish

# Compile to native (produces non-blocking binary with tokio)
cargo run -p tish --features http -- compile examples/async-await/src/main.tish -o async_demo
./async_demo
```

The example uses `fetchAllAsync` to fetch multiple URLs in parallel. The compiled binary uses `#[tokio::main]` and true async `.await`, so I/O does not block the runtime.
