# Tishx Example (JSX for Browser)

Compiles Tish + JSX to plain JavaScript using **Lattish** (the [lattish](https://www.npmjs.com/package/lattish) package): hooks and `createRoot` in source; JSX is compiled for you. Vanilla DOM — no Preact, React, or npm UI libraries.

## Features Used

- **JSX** — Only when compiling to JavaScript (`tish compile --target js`; default `--jsx lattish`)
- **Lattish** — Import `useState`, `createRoot`, etc. from `lattish`; the merged bundle includes the JSX runtime.

## What It Does

- Compiles `App.tish` (with JSX) to `dist/App.js` — self-contained vanilla JS
- Stateful form with Lattish `useState` for name, email, and submit state
- `TextInput` and `App` components (function components + JSX)

## Build

```bash
npm install   # installs lattish
tish compile --target js src/App.tish -o dist/App
```

Or with tish from the repo:

```bash
cargo run -p tish --manifest-path ../../Cargo.toml --release --features full -- compile --target js src/App.tish -o dist/App
```

Or use the npm script: `npm run build`

## Run

Open `index.html` in a browser (requires a local server for ES modules):

```bash
python3 -m http.server 8080
```

Then visit `http://localhost:8080`.

## Project Structure

```
tishx-example/
├── package.json     # lattish dependency
├── tish.yaml
├── index.html       # Loads App.js (runs on load)
├── src/
│   └── App.tish     # Form with stateful components (imports from lattish)
└── dist/            # Generated (gitignored)
    └── App.js
```
