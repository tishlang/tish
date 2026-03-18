# tish_jsx_web

Web-only crate for the Tish JS backend:

- **`LEGACY_DOM_PREAMBLE`** — historical `__h` runtime (standalone `.jsx` or `--jsx legacy`).
- **`VDOM_PRELUDE`** — vnode helpers + `window.__tishactVdomPatch` + `window.__TISH_JSX_VDOM` for `--jsx vdom`.

Native / non-JS compiler targets must not depend on this crate; only `tish_compile_js` pulls it in.

## Vendor runtime

`vendor/Tishact.tish` is a snapshot of the Tishact module from **tish-midi**. Refresh when releasing so CLI-built apps can pin the same runtime.

## Migration from `__h`

| Mode | JSX lowers to | Preamble |
|------|----------------|----------|
| `tishact` (default) | `h(tag, props, [children])` | none — import `h`/`Fragment` from your Tishact module |
| `legacy` | `__h(...)` | full `__h` runtime |
| `vdom` | `__vdom_h(...)` | VDOM prelude; Tishact `createRoot` patches the tree when `window.__TISH_JSX_VDOM` is set |
