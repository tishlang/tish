# Tish npm packages

Publish Tish so users can run it via **npx**:

```bash
npx @tishlang/tish hello.tish
npx @tishlang/tish run src/main.tish
npx @tishlang/tish compile app.tish -o app
npx @tishlang/create-tish-app my-app
```

## Package layout

| Package | Purpose |
|--------|---------|
| `tish/` | Main CLI — single package with binaries for all platforms in `platform/<os>-<arch>/` |
| `create-tish-app/` | Scaffolds a new Tish project |

The main `@tishlang/tish` package ships one npm artifact. It contains a Node wrapper (`bin/tish.js`) and a `platform/` directory with a binary per supported platform (e.g. `platform/darwin-arm64/tish`, `platform/win32-x64/tish.exe`). The wrapper picks the right binary at runtime.

## Building binaries

From the **tish repo root** (parent of `npm/`):

```bash
# Build for your current platform only (e.g. for local test)
./npm/scripts/build-binaries.sh
```

This writes the binary into `npm/tish/platform/<platform>/`. For a full npm release you need binaries for every platform. Either:

- Run the script on each OS/arch (macOS ARM, macOS x64, Linux x64, Linux ARM64, Windows x64), or  
- Use CI (e.g. GitHub Actions matrix) to build each target and copy artifacts into `npm/tish/platform/`.

Example for all platforms (run the appropriate line on each machine or in CI):

```bash
# macOS ARM64
cargo build --release -p tish --target aarch64-apple-darwin
cp target/aarch64-apple-darwin/release/tish npm/tish/platform/darwin-arm64/tish

# macOS x64
cargo build --release -p tish --target x86_64-apple-darwin
cp target/x86_64-apple-darwin/release/tish npm/tish/platform/darwin-x64/tish

# Linux x64
cargo build --release -p tish --target x86_64-unknown-linux-gnu
cp target/x86_64-unknown-linux-gnu/release/tish npm/tish/platform/linux-x64/tish

# Linux ARM64
cargo build --release -p tish --target aarch64-unknown-linux-gnu
cp target/aarch64-unknown-linux-gnu/release/tish npm/tish/platform/linux-arm64/tish

# Windows x64
cargo build --release -p tish --target x86_64-pc-windows-msvc
cp target/x86_64-pc-windows-msvc/release/tish.exe npm/tish/platform/win32-x64/tish.exe
```

## Publishing

1. Ensure `npm/tish/platform/` contains binaries for every supported platform (see above).
2. `npm login` and create the `@tishlang` org on npm if needed.
3. Publish from the package directories:

```bash
cd npm/tish && npm publish --access public
cd ../create-tish-app && npm publish --access public
```

## Local test

After running `./npm/scripts/build-binaries.sh` (so your platform’s binary exists under `npm/tish/platform/`):

```bash
cd npm/tish
node bin/tish.js run /path/to/hello.tish
```

Or link and use npx:

```bash
cd npm/tish && npm link
cd /tmp && npx @tishlang/tish run /path/to/hello.tish
```
