# Async/Await Example

Demonstrates **non-blocking** async/await with **`await fetchAll`** for parallel HTTP fetches.

## Features Used

- `http` — `fetch`, `fetchAll`, `await` (native async `main`)

## Definitive Validation

```bash
cargo test -p tishlangtest_async_parallel_vs_sequential_timing --features http
```

Compiles `parallel.tish` (`await fetchAll`) and `sequential.tish` (`await fetch` in loop), runs both, and **asserts parallel < 60% of sequential time**. Uses httpbin.org/delay/1 (~1s per request): 3 parallel ≈ 1s, 3 sequential ≈ 3s.

## Local Development

```bash
cargo run -p tishlang--features http -- build examples/async-await/src/main.tish -o async_demo
./async_demo
```

## Programs

- `parallel.tish` — `await fetchAll` (3× delay/1)
- `sequential.tish` — `await fetch` in a loop
