# @tishlang/vite-plugin-tish

Official [Vite](https://vitejs.dev) plugin for [Tish](https://tishlang.com). It compiles each
`.tish` file directly into Vite's module graph, so editing a module hot-swaps in place (HMR) instead
of triggering a full page reload, and dev builds carry source maps back to the original `.tish`.

This replaces the common out-of-band shim that ran `tish build` over the whole program and sent
`{ type: 'full-reload' }` on every change.

## Requirements

- Vite 5 or newer.
- The `tish` CLI on your `PATH` (from `@tishlang/tish`), or pass `tishPath`.
- Tish module output (`tish build --target js --format esm`) — the same per-module ESM emit the
  plugin relies on. See [the JS target docs](https://tishlang.com/docs/reference/js-target).

## Install

```bash
npm install -D @tishlang/vite-plugin-tish
```

## Usage

```js
// vite.config.js
import { defineConfig } from "vite";
import tish from "@tishlang/vite-plugin-tish";

export default defineConfig({
  plugins: [tish()],
});
```

Import `.tish` modules as normal ES modules:

```html
<script type="module" src="/src/main.tish"></script>
```

```tish
// src/main.tish
import { makeCounter } from "./counter.tish"

let next = makeCounter(0)
console.log(next())
```

Editing `counter.tish` updates the running app without a full reload, and runtime errors point back
to the `.tish` source in Vite's overlay and the browser debugger.

## Options

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `tishPath` | `string` | `$TISH_PATH` or `tish` | Path to the `tish` binary. |
| `projectRoot` | `string` | Vite `root` | Root for resolving bare specifiers / `node_modules`. |
| `mode` | `"hmr" \| "full-reload"` | `"hmr"` | `hmr` hot-swaps modules; `full-reload` reloads the whole page on any `.tish` change. |

### When to use `full-reload`

For apps compiled to `--target bytecode` (the wasm VM owns all engine state), per-module HMR does not
apply — use `mode: "full-reload"` as the documented fallback.

## How it works

The plugin registers `resolveId` + `load` so each `.tish` import becomes one module in Vite's graph,
compiled via `tish compile-module --target js --format esm --vite-dev`. Relative `.tish` specifiers
are preserved so Vite resolves dependencies through the plugin per module; a self-accepting
`import.meta.hot.accept()` boundary is injected so leaf edits hot-swap. `handleHotUpdate` returns the
changed module node(s) to Vite for per-module invalidation instead of a full reload.

## License

[PIF](https://payitforwardlicense.com/)
