# Async/Await Example

Demonstrates **non-blocking** async/await with `fetchAllAsync` for parallel HTTP fetches.

## Features Used

- `http` - Enables `fetchAsync`, `fetchAllAsync`, and `await`

## Definitive Validation (proves async is non-blocking)

Run the timing test - parallel fetches must complete faster than sequential:

```bash
cargo test -p tish test_async_parallel_vs_sequential_timing --features http
```

This compiles `parallel.tish` (fetchAllAsync) and `sequential.tish` (await in loop), runs both, and **asserts parallel < 60% of sequential time**. Uses httpbin.org/delay/1 (1s per request): 3 parallel ≈ 1s, 3 sequential ≈ 3s.

## Local Development

Build tish with the http feature first, then run or compile:

```bash
# Run with interpreter
cargo run -p tish --features http -- run examples/async-await/src/main.tish

# Compile to native (produces non-blocking binary with tokio)
cargo run -p tish --features http -- compile examples/async-await/src/main.tish -o async_demo
./async_demo
```

## Timing programs

- `parallel.tish` - fetchAllAsync (3x httpbin/delay/1) - runs in parallel
- `sequential.tish` - await fetchAsync in loop - runs one-by-one
