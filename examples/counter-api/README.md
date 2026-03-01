# Counter API

A stateful HTTP API with in-memory counters. Demonstrates state management across requests.

## Features Used

- `http` - Enables the `serve()` function

## Endpoints

| Method | Path | Description |
|--------|------|-------------|
| GET | `/` | API info and available endpoints |
| GET | `/health` | Health check endpoint |
| GET | `/counters` | List all counters and their values |
| GET | `/counter/:name` | Get a specific counter's value |
| POST | `/counter/:name/increment` | Increment a counter by 1 |
| POST | `/counter/:name/decrement` | Decrement a counter by 1 |
| POST | `/counter/:name/reset` | Reset a counter to 0 |

## Example Usage

```bash
# Get counter (auto-creates with value 0)
curl http://localhost:3000/counter/visitors
# {"name":"visitors","value":0}

# Increment
curl -X POST http://localhost:3000/counter/visitors/increment
# {"name":"visitors","value":1}

curl -X POST http://localhost:3000/counter/visitors/increment
# {"name":"visitors","value":2}

# Decrement
curl -X POST http://localhost:3000/counter/visitors/decrement
# {"name":"visitors","value":1}

# List all counters
curl http://localhost:3000/counters
# {"visitors":1}

# Reset
curl -X POST http://localhost:3000/counter/visitors/reset
# {"name":"visitors","value":0}
```

## Local Development

```bash
# Run with interpreter
tish run src/main.tish --features http

# Test incrementing
curl -X POST http://localhost:3000/counter/test/increment
```

## Deploy to Tish Platform

```bash
tish-cli login
tish-cli projects create counter-api
tish-cli link
tish-cli deploy --wait
```

## Note on State

This example uses in-memory state. Counter values are lost when the process restarts. For production use, consider persisting state to a database or file system.
