# Echo Server

An HTTP server that echoes back request details. Useful for testing and debugging HTTP clients.

## Features Used

- `http` - Enables the `serve()` function

## Endpoints

| Method | Path | Description |
|--------|------|-------------|
| ANY | `/echo` | Echoes request as JSON (method, path, headers, body) |
| ANY | `/echo/*` | Same as above, for any sub-path |
| ANY | `/mirror` | Returns the request body with same headers |
| GET | `/health` | Health check endpoint |

## Example Usage

```bash
# Echo a GET request
curl http://localhost:3000/echo
# {"method":"GET","path":"/echo","headers":{},"body":"","timestamp":1234567890}

# Echo a POST with body
curl -X POST -d '{"test": "data"}' http://localhost:3000/echo
# {"method":"POST","path":"/echo","headers":{},"body":"{\"test\": \"data\"}","timestamp":1234567890}

# Mirror endpoint returns your body back
curl -X POST -d 'Hello!' http://localhost:3000/mirror
# Hello!
```

## Local Development

Run without installing tish (from this directory; tish repo is `../..`):

```bash
# Run with interpreter
cargo run -p tish --manifest-path ../../Cargo.toml --release --features http -- run src/main.tish --features http

# Test with curl
curl -X POST -H "Content-Type: application/json" \
  -d '{"message": "Hello Tish!"}' \
  http://localhost:3000/echo
```

Or with tish installed: `tish run src/main.tish --features http`

## Deploy

Deploy with Zectre: `zectre deploy --wait` from this directory. See [Deploy Overview](https://tish-lang.github.io/tish-docs/deploy/overview/) for details.
