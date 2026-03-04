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

## Deploy to Tish Platform

```bash
tish-cli login
tish-cli projects create http-hello
tish-cli link
tish-cli deploy --wait
```

The deployment includes:
- 2 replicas for high availability
- Rolling deployment strategy
- Health check on `/health` endpoint

### Environment Variables

Add env vars in `tish.yaml` under `deploy.env`:

```yaml
deploy:
  replicas: 2
  strategy: rolling
  env:
    LOG_LEVEL: info
```

Or set app-level vars with `tish-cli env add KEY VALUE`. Each process also receives `DEPLOYMENT_ID` and `TASK_ID` from the platform.
