// Mount a built lattish example in jsdom and assert it rendered.
// Usage: node lattish-render.mjs <built-app.js> <expected-substring>
// Exits 0 if #root's rendered text contains the expected substring, else 1.
// This is a real RUNTIME test: the app's createRoot(...).render(App) actually executes,
// the reconciler builds DOM, and we read it back. A build alone would not catch a
// mount-time throw or an empty render.
import { readFileSync } from "node:fs"
import { createRequire } from "node:module"
import { join } from "node:path"

const [, , jsPath, expected] = process.argv
if (!jsPath || expected === undefined) {
  console.error("usage: lattish-render.mjs <app.js> <expected>")
  process.exit(2)
}

// jsdom is CJS and lives in the workspace lattish's node_modules. ESM `import "jsdom"`
// resolves from THIS file's dir (ignores NODE_PATH), so resolve it explicitly relative
// to the lattish package the runner points us at.
const pkg = process.env.LATTISH_PKG
if (!pkg) {
  console.error("LATTISH_PKG not set (path to workspace lattish)")
  process.exit(2)
}
const require = createRequire(join(pkg, "package.json"))
const { JSDOM } = require("jsdom")

// An app's useEffect may kick off async work (e.g. fetch) that has no business in this
// synchronous render check and would reject in jsdom. Swallow it rather than crash.
process.on("unhandledRejection", () => {})
process.on("uncaughtException", () => {})

const dom = new JSDOM(`<!doctype html><html><body><div id="root"></div></body></html>`, {
  pretendToBeVisual: true,
})
// The built bundle is self-contained (lattish inlined) and references the DOM as globals.
globalThis.window = dom.window
globalThis.document = dom.window.document
globalThis.requestAnimationFrame = (cb) => dom.window.setTimeout(() => cb(Date.now()), 0)
globalThis.cancelAnimationFrame = (id) => dom.window.clearTimeout(id)

const root = () => dom.window.document.getElementById("root")
const seen = () => {
  const r = root()
  if (!r) return ""
  return (r.textContent || "") + " " + (r.innerHTML || "")
}
const has = (s) => s.includes(expected)

const code = readFileSync(jsPath, "utf8")
try {
  // top-level createRoot(getElementById("root")).render(App) runs here (synchronous mount)
  ;(0, eval)(code) // eslint-disable-line no-eval
} catch (e) {
  console.error("RENDER THREW:", e && e.stack ? e.stack.split("\n").slice(0, 3).join("\n") : e)
  process.exit(1)
}

// 1) assert on the SYNCHRONOUS initial render (what the user sees before effects fire)
if (has(seen())) {
  console.error(`rendered OK (#root contains "${expected}")`)
  process.exit(0)
}
// 2) not there yet: let microtask/timeout effects settle, then re-check
await new Promise((r) => dom.window.setTimeout(r, 40))
const after = seen()
if (after.replace(/ /g, "").trim() === "") {
  console.error("EMPTY RENDER: #root has no content after mount")
  process.exit(1)
}
if (!has(after)) {
  console.error(`MISSING: expected "${expected}" in rendered #root, got: ${JSON.stringify(root().textContent.slice(0, 180))}`)
  process.exit(1)
}
console.error(`rendered OK (#root contains "${expected}")`)
process.exit(0)
