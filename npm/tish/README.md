# @tishlang/tish

[Tish](https://github.com/tishlang/tish) is a minimal TypeScript/JavaScript–compatible language: run with an interpreter, use a REPL, or build native binaries and other targets.

This npm package ships the **Tish CLI**. It includes platform-specific native binaries under `platform/`; **`npm install`** runs `postinstall`, which copies the correct binary to `bin/tish`. That file is what runs when you invoke `tish` — the CLI itself is native, not Node. Node **22+** is required for install scripts and tooling (e.g. semantic-release in this repo); the `tish` binary has no Node runtime dependency.

## Install

```bash
npm install @tishlang/tish
```

Or run without installing:

```bash
npx @tishlang/tish --help
```

## Quick start

Run a `.tish` file (shorthand: first argument is treated as a file → `run`):

```bash
npx @tishlang/tish hello.tish
npx @tishlang/tish run src/main.tish
```

Build a native executable:

```bash
npx @tishlang/tish build app.tish -o app
./app
```

Native builds use the Rust backend by default (requires [Rust](https://rustup.rs) and `cargo` on your PATH). The package includes the Tish workspace source (`Cargo.toml`, `crates/`, `justfile`) so `tish build` can run `cargo build` for your program. For pure Tish without native imports, use `--native-backend cranelift` (no Rust toolchain needed).

Start the REPL:

```bash
npx @tishlang/tish repl
```

## Supported platforms

Prebuilt binaries are included for:

- `darwin-arm64`, `darwin-x64`
- `linux-x64`, `linux-arm64`
- `win32-x64`

If your platform is missing, [build from source](https://github.com/tishlang/tish).

## Documentation

- Repository: <https://github.com/tishlang/tish>
- User docs: <https://github.com/tishlang/tish-docs>

## Scaffold a project

```bash
npx @tishlang/create-tish-app my-app
```

## License

See the [Tish repository LICENSE](https://github.com/tishlang/tish/blob/main/LICENSE) (Pay It Forward).
