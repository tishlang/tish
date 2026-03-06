# HTTP Modules Example

Demonstrates import/export with an HTTP server. Uses `serve` for deployment-ready responses.

## Layout

```
src/
├── main.tish    # Entry point with serve, imports greet
└── greet.tish   # Exports the greet function
```

## Run locally

Build tish with http and process features, then:

```bash
tish run src/main.tish
```

Or compile:

```bash
tish compile src/main.tish -o server --feature http --feature process
./server
```

Then visit `http://localhost:8080` for JSON response with the greeting.

## Deploy

Use the tish-dev CLI from this directory. See [Deploy Overview](/deploy/overview/).
