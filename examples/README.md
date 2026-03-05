# Tish Examples

Ready-to-deploy example applications for the Tish Platform.

## Examples

| Example | Description | Features |
|---------|-------------|----------|
| [hello-world](hello-world/) | Minimal app that logs and exits | None |
| [http-hello](http-hello/) | Basic HTTP server with routing | `http` |
| [json-api](json-api/) | REST API with JSON responses | `http` |
| [echo-server](echo-server/) | Echoes request details back | `http` |
| [counter-api](counter-api/) | Stateful counter service | `http` |
| [async-await](async-await/) | Async/await with fetchAsync | `http` |

## Prerequisites

1. **Tish** – compiler/runtime (this repo). Build for the platform build service: `cargo build --release -p tish`
2. **Tish Dev CLI** – deploy from the **tish-dev** repo via `cargo run -p tish-cli --` (no install).
3. **Tish Platform** – running locally, e.g. `just run-all` in tish-platform. Config must point at this tish repo: `tish_compiler_path` and `tish_workspace_path` in platform’s `config/default.toml`.

## Deploy examples (tish-dev CLI, no install)

Deploy code from this repo’s examples using the tish-dev CLI run from source. Do **not** install tish-cli; run it with `cargo run -p tish-cli --manifest-path <tish-dev>/Cargo.toml --`.

1. **Start platform** (in tish-platform): `just run-all`. Set `tish_compiler_path` and `tish_workspace_path` in `config/default.toml` to this tish repo.

2. **Build tish** (in this repo): `cargo build --release -p tish`.

3. **Deploy an example** – from this repo, in the example directory (tish and tish-dev siblings):
   ```bash
   cd examples/http-hello
   cargo run -p tish-cli --manifest-path ../../tish-dev/Cargo.toml -- login
   cargo run -p tish-cli --manifest-path ../../tish-dev/Cargo.toml -- projects create http-hello
   cargo run -p tish-cli --manifest-path ../../tish-dev/Cargo.toml -- link
   cargo run -p tish-cli --manifest-path ../../tish-dev/Cargo.toml -- deploy --wait
   ```
   For another example, use its name (e.g. `echo-server`, `counter-api`) and `cd` to that example dir before `link` and `deploy`. API URL defaults to `http://localhost:47080`; use `TISH_API_URL` or `--api-url` to override. If you have an API key: `-- login --with-key YOUR_KEY`.

## Quick Start

### Local Development

Run any example locally with the interpreter:

```bash
cd examples/http-hello
tish run src/main.tish --features http
```

Or compile to a native binary:

```bash
tish compile src/main.tish -o server --features http
./server
```

### Deploy to Tish Platform

Use the **tish-dev** CLI from source (no install). From an example directory, prefix every command with:

`cargo run -p tish-cli --manifest-path ../../tish-dev/Cargo.toml --`

1. **Authenticate**: `... -- login` (or `... -- login --with-key YOUR_KEY`)
2. **Create project**: `... -- projects create http-hello` (use the example name)
3. **Link**: `... -- link`
4. **Deploy**: `... -- deploy --wait`
5. **Logs**: `... -- status`, `... -- logs <task-id>`

See **Deploy examples (tish-dev CLI, no install)** above for the full sequence.

## Project Structure

Each example follows this structure:

```
example-name/
├── tish.yaml        # Deployment configuration
├── README.md        # Example documentation
└── src/
    └── main.tish    # Entry point
```

## Configuration Reference

The `tish.yaml` file configures how your app is built and deployed:

```yaml
name: my-app                    # Required: project name

build:
  source: ./src/main.tish       # Entry point (default: ./src/main.tish)
  features:                     # Feature flags to enable
    - http                      # Network access (fetch, serve)
    - fs                        # File system access
    - process                   # Process control

deploy:
  replicas: 1                   # Number of instances (default: 1)
  strategy: rolling             # Deployment strategy

resources:
  cpu: 100m                     # CPU limit
  memory: 128Mi                 # Memory limit

networking:
  port: 3000                    # Port to expose
  protocol: http                # Protocol (http, tcp, grpc)
  health_check:
    path: /health               # Health check endpoint
    interval: 10s               # Check interval
    timeout: 5s                 # Request timeout
```

## Feature Flags

| Flag | Enables |
|------|---------|
| `http` | Network access (`fetch`, `fetchAll`, `serve`) |
| `fs` | File system (`readFile`, `writeFile`, `mkdir`, etc.) |
| `process` | Process control (`process.exit`, `process.env`, etc.) |
| `regex` | Regular expressions (`RegExp`, `String.match`, etc.) |
| `full` | All features |

By default, Tish runs in **secure mode** with no features enabled.

## CLI Commands Reference (tish-dev, no install)

Run from an example dir: `cargo run -p tish-cli --manifest-path ../../tish-dev/Cargo.toml -- <command>`

```bash
# Authentication
-- login                    # Browser-based login
-- login --with-key KEY     # Use API key
-- logout                   # Clear credentials
-- whoami                   # Show current user

# Projects
-- projects list            # List all projects
-- projects create NAME     # Create new project
-- projects delete NAME     # Delete a project

# Deployment
-- link                     # Link directory to project
-- deploy                   # Deploy application
-- deploy --wait            # Deploy and wait for completion
-- deploy --prod            # Production deployment

# Monitoring
-- status                   # Show deployment status
-- logs <task-id>           # View task logs

# Environment Variables
-- env list                 # List env vars
-- env add KEY=value        # Add env var
-- env rm KEY               # Remove env var
```
