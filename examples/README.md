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

## Prerequisites

1. **Tish CLI** - Install the Tish compiler/interpreter:
   ```bash
   cargo build --release -p tish
   ```

2. **Tish Dev CLI** - Install the deployment CLI:
   ```bash
   cargo install --path ../tish-dev/crates/tish-cli
   ```

3. **Tish Platform** - A running instance of tish-platform

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

1. **Authenticate** with the platform:
   ```bash
   tish-cli login
   ```

2. **Create a project** on the platform:
   ```bash
   cd examples/http-hello
   tish-cli projects create http-hello
   ```

3. **Link** your local directory to the project:
   ```bash
   tish-cli link
   ```

4. **Deploy** the application:
   ```bash
   tish-cli deploy --wait
   ```

5. **View logs** (optional):
   ```bash
   tish-cli status
   tish-cli logs <task-id>
   ```

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

## CLI Commands Reference

```bash
# Authentication
tish-cli login              # Browser-based login
tish-cli logout             # Clear credentials
tish-cli whoami             # Show current user

# Projects
tish-cli projects list      # List all projects
tish-cli projects create    # Create new project
tish-cli projects delete    # Delete a project

# Deployment
tish-cli link               # Link directory to project
tish-cli deploy             # Deploy application
tish-cli deploy --wait      # Deploy and wait for completion
tish-cli deploy --prod      # Production deployment

# Monitoring
tish-cli status             # Show deployment status
tish-cli logs <task-id>     # View task logs

# Environment Variables
tish-cli env list           # List env vars
tish-cli env add KEY=value  # Add env var
tish-cli env rm KEY         # Remove env var
```
