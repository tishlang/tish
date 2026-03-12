# Tishx Example (JSX for Browser)

Compiles Tish + JSX to plain JavaScript. Uses vanilla DOM only — no Preact, React, or any 3rd party modules.

## Features Used

- **JSX** — Only supported when compiling to JavaScript (`tish compile --target js`)
- **useState** — React-like hook for stateful components (see `hooks.tish`)

## What It Does

- Compiles `App.tish` (with JSX) to `dist/App.js` — self-contained vanilla JS
- Stateful form with `useState` for name, email, and submit state
- `TextInput` component and `App` component with hooks

## Build

```bash
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
├── tish.yaml
├── index.html      # Loads App.js (runs on load)
├── src/
│   ├── hooks.tish  # useState, createRoot (React-like)
│   └── App.tish    # Form with stateful components
└── dist/           # Generated (gitignored)
    └── App.js
```
