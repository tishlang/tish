# HTTP + environment variables

Small HTTP server like [http-hello](../http-hello/), but reads **`process.env`** so you can verify config and secrets injected at runtime (local shells, containers, or Zectre deployments).

## Features used

- `http` — `serve()` and request handling
- `process` — `process.env` for `PORT`, `TEST`, and `DEPLOYMENT_ID`

## Endpoints

| Path | Description |
|------|-------------|
| `GET /` | JSON: greeting, `env_var_test` (`process.env.TEST`), `deployment_id` (`process.env.DEPLOYMENT_ID` or `""`) |
| `GET /health` | Plain `OK` (used by `tish.yaml` health check) |

## Environment variables

| Variable | Role |
|----------|------|
| `PORT` | Listen port (default **8080** if unset) |
| `TEST` | Exposed in the `/` JSON as `env_var_test` (demo arbitrary env) |
| `DEPLOYMENT_ID` | Exposed in the `/` JSON as `deployment_id` when set |

Example:

```bash
export PORT=3000
export TEST=local-dev
export DEPLOYMENT_ID=my-deployment
tish run src/main.tish --features http,process
# curl http://127.0.0.1:3000/
```

## Local development

From this directory (tish repo root is `../..`):

```bash
cargo run -p tishlang --manifest-path ../../Cargo.toml --release --features http,process -- run src/main.tish --features http,process
```

Or with `tish` installed:

```bash
tish run src/main.tish --features http,process
```

## Deploy

Includes [`tish.yaml`](./tish.yaml): HTTP app, `/health` check, two replicas, rolling strategy. Deploy with Zectre from this directory. See [Deploy overview](https://tishlang.com/docs/deploy/overview/).
