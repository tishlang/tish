# JavaScript target (`tish build --target js`)

Tish compiles to JavaScript in one of two output formats, selected with `--format`.

| Format | Output | Use case |
|--------|--------|----------|
| `bundle` (default) | One merged `.js` file (`-o app.js`) | `<script>` tags, CLI tools, embeds without a bundler |
| `esm` | One `.js` file per `.tish` module under a directory (`-o dist/`), with real `import`/`export` | Vite/Rollup/esbuild production builds (tree-shaking, code-splitting) |

> In `esm` mode the build prints the entry module's output path (`Entry: …`) — hand that file to your bundler. It may live in a subdirectory of `-o` (see [output layout](#output-layout)).

## Bundle format

```bash
tish build app.tish -o app.js --target js          # implicitly --format bundle
```

All statically-imported modules are resolved and merged into a single flat program before JavaScript is emitted, so the output has no `import`/`export` statements (an entry `export default` is the one exception). This is simple to load but opaque to a bundler: there is no module graph to tree-shake, and two modules that export the same top-level name collide in the single shared scope.

## ESM format

```bash
tish build src/main.tish -o dist --target js --format esm
```

Each reachable `.tish` module is compiled to its own `.js` file, preserving the source tree layout, with real ES `import`/`export`.

### Output layout

The output tree is rooted at the **deepest directory common to every module in the graph** (entry plus all of its transitive dependencies), and mirrors the real filesystem beneath it. For a self-contained project that common base is just the project root, so the layout is the obvious one:

```
# entry: src/main.tish, all deps under src/
src/workbench/boot.tish        ->  dist/workbench/boot.js
src/internal/layout/index.tish ->  dist/internal/layout/index.js
```

When the graph also pulls in modules **outside** the entry's package — a sibling package or a `.tish` library in `node_modules` — the common base moves up to the nearest shared ancestor so those modules get a stable home too. The entry then lands in a subtree (the `Entry: …` line tells you where):

```
# entry: apps/ide/src/main.tish, deps in node_modules/ and packages/
apps/ide/src/main.tish          ->  dist/apps/ide/src/main.js   (← Entry)
node_modules/lattish/src/Lattish.tish -> dist/node_modules/lattish/src/Lattish.js
packages/memory/schema.tish     ->  dist/packages/memory/schema.js
```

A bare specifier (`import { h } from "lattish"`) is resolved to its `.tish` entry and rewritten to a **relative** `.js` path into that mirrored tree (e.g. `../../../node_modules/lattish/src/Lattish.js`), so the emitted graph is self-contained and a bundler can follow it without any module resolution config.

Because every module keeps its own scope:

- A bundler (Vite, Rollup, esbuild) sees a static `import` graph and can **tree-shake** unused exports, **code-split**, scope-hoist, and minify.
- Two modules can export the **same name** without colliding (issue #282). For example `a.tish` and `b.tish` may both `export fn activate`, imported under distinct local aliases:

```javascript
import { activate as activateA } from "./a.tish"
import { activate as activateB } from "./b.tish"
```

Relative import specifiers keep their shape with `.tish` rewritten to `.js` (`./dep.tish` becomes `./dep.js`), since the output tree mirrors the source tree.

### Vite production recipe

Compile Tish to ESM, then let Vite own minification and tree-shaking:

```bash
tish build src/main.tish -o dist/tish --target js --format esm
# the build prints `Entry: dist/tish/<…>/main.js` — point Vite at that file as an entry;
# Vite bundles + tree-shakes the graph
```

### Limitations (current)

- `-o` is treated as a **directory** in ESM mode.
- **Native imports** (`tish:*`, `cargo:*`, `ffi:*`, and the built-in `fs`/`http`/`process`/`ws`) are rejected — they require `--target native`.
- When the graph spans modules outside the entry's package, the output tree is rooted at their nearest common ancestor, so the entry is emitted in a subtree (the `Entry: …` line reports its path) and the tree may include `node_modules/` / sibling-package directories.
- `--source-map` is **bundle-only** for now; ESM per-file source maps are a follow-up.
- `.jsx` / `.js` single-file inputs are bundle-only.
