# Tish npm packages

Publish Tish so users can run it via **npx**:

```bash
npx @tishlang/tish hello.tish
npx @tishlang/tish run src/main.tish
npx @tishlang/tish build app.tish -o app
npx @tishlang/create-tish-app my-app
```

## Package layout

| Package | Purpose |
|--------|---------|
| `tish/` | Main CLI — single package with binaries for all platforms in `platform/<os>-<arch>/` |
| `create-tish-app/` | Scaffolds a new Tish project |

The main `@tishlang/tish` package ships one npm artifact. It includes a `platform/` directory with a binary per supported platform (e.g. `platform/darwin-arm64/tish`, `platform/win32-x64/tish.exe`). On **`npm install`**, `postinstall` copies the binary for your OS/arch to `bin/tish`, which is the `tish` command — no Node process per invocation. If you use **`npm install --ignore-scripts`**, run `node scripts/install-bin.js` inside the package or `bin/tish` will be missing. When developing the repo without running `./npm/scripts/build-binaries.sh` first, `postinstall` skips copying and prints a warning (install still succeeds); run `build-binaries.sh` then `node npm/tish/scripts/install-bin.js` to materialize `bin/tish`.

## Building binaries

From the **tish repo root** (parent of `npm/`):

```bash
# Build for your current platform only (e.g. for local test)
./npm/scripts/build-binaries.sh
```

This writes the binary into `npm/tish/platform/<platform>/`. For a full npm release you need binaries for every platform. Either:

- Run the script on each OS/arch (macOS ARM, macOS x64, Linux x64, Linux ARM64, Windows x64), or  
- Use the **GitHub Actions workflow** (see below): on every push/PR to `main`, the workflow builds all platforms and uploads the **`npm-tish-platform-binaries`** artifact. Download it from the Actions run and extract into `npm/tish/platform/` to get a full set for publishing.

Example for all platforms (run the appropriate line on each machine or in CI):

```bash
# macOS ARM64
cargo build --release -p tishlang--target aarch64-apple-darwin
cp target/aarch64-apple-darwin/release/tish npm/tish/platform/darwin-arm64/tish

# macOS x64
cargo build --release -p tishlang--target x86_64-apple-darwin
cp target/x86_64-apple-darwin/release/tish npm/tish/platform/darwin-x64/tish

# Linux x64
cargo build --release -p tishlang--target x86_64-unknown-linux-gnu
cp target/x86_64-unknown-linux-gnu/release/tish npm/tish/platform/linux-x64/tish

# Linux ARM64
cargo build --release -p tishlang--target aarch64-unknown-linux-gnu
cp target/aarch64-unknown-linux-gnu/release/tish npm/tish/platform/linux-arm64/tish

# Windows x64
cargo build --release -p tishlang--target x86_64-pc-windows-msvc
cp target/x86_64-pc-windows-msvc/release/tish.exe npm/tish/platform/win32-x64/tish.exe
```

## GitHub Actions

The workflow [`.github/workflows/build-npm-binaries.yml`](../.github/workflows/build-npm-binaries.yml) runs on push and PR to `main`. It:

- **Test & coverage** (Ubuntu): runs `cargo nextest run -p tishlang--features full`, publishes **test results** (JUnit) to the Actions summary, and generates **code coverage** via `cargo llvm-cov`. Artifacts: `coverage-lcov` (lcov.info), `coverage-html` (browsable report).
- **Build**: builds the CLI for
  - `linux-x64`, `linux-arm64` (Ubuntu; ARM64 via [cross](https://github.com/cross-rs/cross))
  - `darwin-arm64`, `darwin-x64` (macOS 14)
  - `win32-x64` (Windows)

The **assemble** job produces a single artifact **`npm-tish-platform-binaries`**: the full `npm/tish/platform/` directory. Download it from the workflow run and extract into your repo (or into a clean `npm/tish/` tree) before publishing.

## Automated releases (main branch)

On **push to `main`**, the CI workflow runs a **release** job that:

1. **Semantic versioning** — Uses [semantic-release](https://github.com/semantic-release/semantic-release) and [conventional commits](https://www.conventionalcommits.org/) to compute the next version:
   - `fix:` or `fix(scope):` → patch
   - `feat:` or `feat(scope):` → minor
   - `BREAKING CHANGE:` or `feat!:` / `fix!:` → major
2. **GitHub release** — Creates a GitHub release with generated notes and attaches **tish-platform-binaries.zip** (all platform binaries).
3. **npm publish** — When **`NPM_TOKEN`** is configured as a repository secret, publishes `@tishlang/tish` and `@tishlang/create-tish-app`. The workflow also **attempts** to publish the unscoped package **`create-tish-app`** (so `npx create-tish-app` works without a scope). That second publish is **non-fatal**: npm often returns **403** if the token is a **granular** token limited to the `@tishlang` scope, or if your npm account does not own the unscoped package name. Fix by using a token with publish rights on **`create-tish-app`**, or by transferring/claiming that package on npm under the same maintainer as `@tishlang`.

To enable npm publishing: create an [npm access token](https://www.npmjs.com/settings/~/tokens) (automation type), then add it as a secret named **`NPM_TOKEN`** in the repo (Settings → Secrets and variables → Actions). The release job will skip npm publish if the secret is missing (GitHub release still runs).

### First-time unscoped package `create-tish-app`

The unscoped name **`create-tish-app`** is a **separate** npm package from **`@tishlang/create-tish-app`**. npm **creates** it on the **first successful publish** of that name under your account.

1. Use an **`NPM_TOKEN`** that is allowed to **publish new public packages** for your npm user (a **classic** [automation token](https://docs.npmjs.com/creating-and-viewing-access-tokens) is the usual choice for the first publish). Some **granular** tokens only list existing packages and return **403** until the package exists — chicken and egg.
2. In GitHub: **Actions** → workflow **“Bootstrap unscoped create-tish-app on npm”** → **Run workflow** → enter a **version** (e.g. `1.3.2`) that is **not** already published for `create-tish-app`.
3. After the package exists, you can switch to a **granular** token that explicitly includes **`create-tish-app`** if you prefer; the normal [NPM release](../.github/workflows/npm-release.yml) job will then publish both scoped and unscoped packages.

If the name is already owned by another npm user, you cannot claim it with your token — keep using **`npx @tishlang/create-tish-app`** only.

## Manual publishing

1. Ensure `npm/tish/platform/` contains binaries for every supported platform (from CI artifact or local builds).
2. Copy workspace source for native compile (rust backend): from repo root, run:
   ```bash
   cp Cargo.toml LICENSE justfile npm/tish/
   cp -r crates npm/tish/
   ```
3. `npm login` and create the `@tishlang` org on npm if needed.
4. Publish from the package directories:

```bash
cd npm/tish && npm publish --access public
cd ../create-tish-app && npm publish --access public
node -e "const fs=require('fs');const p=require('./package.json');p.name='create-tish-app';fs.writeFileSync('package.json',JSON.stringify(p,null,2));"
npm publish --access public
git checkout package.json
```

## Local test

After running `./npm/scripts/build-binaries.sh` (so your platform’s binary exists under `npm/tish/platform/`):

```bash
cd npm/tish
node scripts/install-bin.js
./bin/tish run /path/to/hello.tish
```

Or link and use npx:

```bash
cd npm/tish && npm link
cd /tmp && npx @tishlang/tish run /path/to/hello.tish
```
