# Tish Examples

Example applications you can run and compile with Tish. Build the CLI from the repo root: `cargo build --release -p tish`.

## Examples

| Example | Description | Features |
|---------|-------------|----------|
| [npm-usage](npm-usage/) | Uses `@tishlang/tish` from npm (`npm install`, scripts) | None |
| [hello-world](hello-world/) | Minimal app that logs and exits | None |
| [matmul](matmul/) | Dense matrix multiply across backends: CPU (f64 native), Metal GPU (MPS via Swift), Apple MLX (Metal via Python) | `fs`, `process` (GPU/MLX variants) |
| [new-expression](new-expression/) | `new Uint8Array` / `new AudioContext` on every target; real `new` on `--target js` | None |
| [http-hello](http-hello/) | Basic HTTP server with routing | `http` |
| [http-env-var-example](http-env-var-example/) | HTTP server reading `PORT`, `TEST`, `DEPLOYMENT_ID` from `process.env` | `http`, `process` |
| [json-api](json-api/) | REST API with JSON responses | `http` |
| [echo-server](echo-server/) | Echoes request details back | `http` |
| [counter-api](counter-api/) | Stateful counter service | `http` |
| [async-await](async-await/) | Async/await with `fetch` / `fetchAll` | `http` |
| [mdx-docs](mdx-docs/) | Static docs: MDX, file-based routing, pre-rendered | `http`, `fs` |
| [json-file-edit](json-file-edit/) | Read JSON file, decode, modify, write back | `fs` |
| [tishx-example](tishx-example/) | Tish + JSX compiled to vanilla JavaScript (no 3rd party libs) | — |

## Quick start — run locally

From the repo root, run any example with the interpreter:

```bash
cd examples/http-hello
tish run src/main.tish --feature http
```

Or compile to a native binary:

```bash
tish build src/main.tish -o server --feature http
./server
```

Use the same pattern for other examples; enable the features they need (e.g. `http`, `fs`, `process`).

## Deploy

Use the **zectre** CLI from an example directory. See [Deploy Overview](https://tishlang.com/docs/deploy/overview/) for details. Prerequisites: built `tish` binary and platform config pointing at this repo.

## Project structure

Each example follows:

```
example-name/
├── zectre.yaml      # Optional: Zectre deploy / build manifest
├── README.md        # Example-specific docs
└── src/
    └── main.tish    # Entry point
```

## Feature flags

| Flag | Enables |
|------|---------|
| `http` | Network access (`fetch`, `fetchAll`, `serve`) |
| `fs` | File system (`readFile`, `writeFile`, `mkdir`, etc.) |
| `process` | Process control (`process.exit`, `process.env`, etc.) |
| `regex` | Regular expressions (`RegExp`, `String.match`, etc.) |
| `full` | All features |

By default, Tish runs in **secure mode** with no features enabled. Pass `--features http` (or other flags) when running or compiling.
