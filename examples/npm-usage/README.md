# npm usage

Shows how to depend on **`@tishlang/tish`** from the [npm registry](https://www.npmjs.com/package/@tishlang/tish) and run or compile Tish from **npm scripts** (uses `node_modules/.bin/tish`).

## Prerequisites

- Node.js **22+**
- npm (or another client that respects `package.json` `scripts` and `dependencies`)

## Setup

From this directory:

```bash
npm install
```

This installs `@tishlang/tish`, which provides the `tish` CLI for your platform.

## Run (interpreter)

```bash
npm start
# same as: npx tish run src/main.tish
```

Or without a prior install:

```bash
npx @tishlang/tish run src/main.tish
```

## Compile to native

```bash
npm run compile
./app
```

On Windows, run `app.exe` instead of `./app`.

## REPL

```bash
npm run repl
```

## Version pin

`package.json` uses `"@tishlang/tish": "^1.0.0"` so installs get compatible 1.x releases (e.g. [1.0.7](https://www.npmjs.com/package/@tishlang/tish/v/1.0.7)). Adjust the range if you need a specific version.

## Developing inside the Tish repo

Examples in this repo are usually run with a **cargo-built** `tish`. This example is meant for **published npm** usage. To try it locally against your workspace package instead of the registry, you can use `npm link` from `npm/tish` or a `file:` dependency — that is optional and not required for end users.
