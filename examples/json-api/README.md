# JSON API

An HTTP server returning JSON responses with proper content-type headers.

## Features Used

- `http` - Enables the `serve()` function

## Endpoints

| Method | Path | Description |
|--------|------|-------------|
| GET | `/` | API info and available endpoints |
| GET | `/health` | Health check (returns JSON) |
| GET | `/users` | List all users |
| GET | `/users/:id` | Get user by ID |

## Example Responses

```bash
# Get API info
curl http://localhost:3000/
# {"name":"JSON API Example","version":"1.0.0","endpoints":["/users","/users/:id","/health"]}

# List users
curl http://localhost:3000/users
# [{"id":1,"name":"Alice","email":"alice@example.com"},...]

# Get single user
curl http://localhost:3000/users/1
# {"id":1,"name":"Alice","email":"alice@example.com"}
```

## Local Development

Run without installing tish (from this directory; tish repo is `../..`):

```bash
# Run with interpreter
cargo run -p tishlang--manifest-path ../../Cargo.toml --release --features http -- run src/main.tish --features http

# Then test: curl http://localhost:3000/users
```

Or with tish installed: `tish run src/main.tish --features http`

## Deploy

Deploy with Zectre: `zectre deploy --wait` from this directory. See [Deploy Overview](https://tishlang.github.io/tish-docs/deploy/overview/) for details.
