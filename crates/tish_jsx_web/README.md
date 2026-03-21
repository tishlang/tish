# tish_jsx_web

Web-only crate for the Tish JS backend:

- **`VDOM_PRELUDE`** — vnode helpers + `window.__lattishVdomPatch` + `window.__LATTISH_JSX_VDOM` for `--jsx vdom`.

Native / non-JS compiler targets must not depend on this crate; only `tish_compile_js` pulls it in.

## Vendor runtime

`vendor/Lattish.tish` is a copy refreshed from the **lattish** npm package. Run `just refresh-lattish` to update from the sibling lattish package.

## JSX modes (`--jsx`)

| Mode | JSX lowers to | Preamble |
|------|----------------|----------|
| `lattish` (default) | Lattish-style JSX lowering | none — merge `Lattish.tish` by importing any export you need (or `import {} from "lattish"` for JSX-only) |
| `vdom` | `__vdom_h(...)` | VDOM prelude; Lattish `createRoot` patches the tree when `window.__LATTISH_JSX_VDOM` is set |
