# HTTP Hello

A basic HTTP server demonstrating the `serve()` function with simple routing.

## Features Used

- `http` - Enables the `serve()` function

## Endpoints

| Path | Description |
|------|-------------|
| `GET /` | Returns greeting message |
| `GET /health` | Health check endpoint |
| `GET /about` | Returns version info |

## Local Development

Run without installing tish (from this directory; tish repo is `../..`):

```bash
# Run with interpreter (requires http feature)
cargo run -p tish --manifest-path ../../Cargo.toml --release --features http -- run src/main.tish --features http

# Then visit: http://localhost:3000/
```

Or with tish installed: `tish run src/main.tish --features http`

## Deploy

Deploy with Zectre: `zectre deploy --wait` from this directory. See [Deploy Overview](https://tish-lang.github.io/tish-docs/deploy/overview/) for details.
