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

```bash
# Run with interpreter
tish run src/main.tish --features http

# Then test: curl http://localhost:3000/users
```

## Deploy to Tish Platform

```bash
tish-cli login
tish-cli projects create json-api
tish-cli link
tish-cli deploy --wait
```
