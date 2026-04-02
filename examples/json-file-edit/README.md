# JSON File Edit Example

Reads a JSON file, decodes it with `JSON.parse`, modifies the data, then writes it back with `JSON.stringify` and `writeFile`.

## Features Used

- `fs` – file system access (`readFile`, `writeFile`)

## What It Does

1. **Read** – `readFile(data.json)` loads the raw string
2. **Decode** – `JSON.parse(raw)` turns the string into an object
3. **Modify** – Increments `version`, updates `updated`, appends an item, adds `lastRun`
4. **Write back** – `JSON.stringify(data)` then `writeFile(data.json, output)`
5. **Verify** – Reads the file again to confirm changes

## Local Development

Run from this directory (tish repo is `../..`). File I/O requires the `fs` feature and the interpreter backend:

```bash
# Run with interpreter (fs feature required for readFile/writeFile)
cargo run -p tishlang--manifest-path ../../Cargo.toml --release --features fs -- run src/main.tish --backend interp

# Or with tish installed
tish run src/main.tish --backend interp
```
(Ensure tish was built with `--features fs` or `--features full`.)

To compile to a native binary (includes fs when built with full):

```bash
cargo run -p tishlang--manifest-path ../../Cargo.toml --release --features full -- build src/main.tish -o json-file-edit
./json-file-edit
```

## Files

- `src/main.tish` – entry point
- `data.json` – JSON file (created with defaults if missing)
