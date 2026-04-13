# Tish language reference

**Canonical spec** for tools and LLMs. Single source of truth for syntax and semantics; implementation lives in `crates/tish_lexer`, `crates/tish_parser`, `crates/tish_ast`.

Tish is a minimal JS/TS-like language: same source runs in a **tree-walking interpreter**, a **bytecode VM**, or **compiled targets** — Rust transpilation linked to **`tishlang_runtime`**, standalone binaries that **embed bytecode and run the same VM**, **WASM/WASI**, or **JavaScript**. See **Native compile (implementation status)** for what each path does today versus the long-term goal (primitive lowering, true AOT). **Secure-by-default:** network, filesystem, and process APIs are feature-gated.

**No `undefined`** — use `null`. **`typeof null`** is `"null"` (not `"object"`). **Strict equality only:** `===` / `!==`. **`let` / `const` only** (no `var`). **No `this`**, prototypes, or `class` / `super`. Plain objects and arrays. **`?.` yields `null`**, not undefined. Parser also accepts **`new`** expressions (no class syntax; uncommon in idiomatic Tish).

---

## Syntax (compact)

**Keywords:** `fn` / `function`, `let`, `const`, `if` `else`, `while`, `do` `while`, `for`, `switch` `case` `default`, `return` `break` `continue`, `try` `catch` `throw`, `async` `await`, `import` `export`, `new`, `typeof`, `void`, `true` `false` `null`.

**Literals:** numbers; strings `"`/`'` (escapes `\n` `\r` `\t` `\\` `\"` `\'`); arrays `[]`; objects `{ k: v }` (fixed keys at parse time); template literals `` `x ${e} y` ``; JSX supported in lexer.

**Operators:** `+` (add/concat), `-` `*` `/` `%` `**`; bitwise `&` `|` `^` `~` `<<` `>>` (32-bit int semantics); compare `<` `<=` `>` `>=`; logical `&&` `||` `!`; ternary `? :`; `??`; `?.`; compound assign `+=` `-=` …; postfix `++` `--` on identifiers.

**Functions:** `fn name(a, b) { … }`, single-expr body `fn f(x) = x * 2`, arrows `let g = (a, b) => a + b`, `async fn …` with `await`.

**Control flow:** `if`/`else`, `while`, `do`/`while`, C-style `for`, `for (let|const x of arr)` (arrays/strings), `switch`, `try`/`catch`.

**Blocks:** `{ … }` **or** indentation (lexer emits `Indent`/`Dedent`). **1 tab = 1 level; 2 spaces = 1 level.**

**Modules:** `import { a } from 'http'`; native `tish:fs`, `tish:http`, `tish:process`, `tish:ws`, etc. (Rust backend only). **`cargo:…`** — import Rust crates by Cargo package name, e.g. `import { to_string } from 'cargo:tish_serde_json'`. Requires project `package.json` → `tish.rustDependencies` to declare the same key (version string or **`path`** to a local crate). The crate you list in `rustDependencies` must expose each imported name as **`pub fn {snake_case}(args: &[Value]) -> Value`** (using `tishlang_runtime::Value`). **Phase 1 (today):** use **`tishlang-cargo-bindgen`** (`tishlang_cargo_bindgen`): from **`package.json`** it reads **`tish.rustDependencies`** (path to the glue crate), reads the **upstream** crate + semver from that glue crate’s **`Cargo.toml`** if present, otherwise from the **project root** **`Cargo.toml`** (registry deps in **`[dependencies]`** / **`[dev-dependencies]`**; **`tishlang_core`** path deps are skipped), then runs **`cargo metadata`** on the upstream, scans **`src/**/*.rs`** with **`syn`**, classifies matching **`pub fn`** signatures, and writes the glue **`Cargo.toml`**. Use **`--tishlang-runtime-path`** (or env) so glue **`tishlang_runtime`** matches **`tish build`**; a crates.io **`tishlang_runtime`** line in glue can pull a second **`tishlang_core`** and break with conflicting **`Value`** types. You can still pass **`--dependency`** / **`--out-dir`** for non-`package.json` flows (see **`tish-cargo-example`** / `npm run gen:bindings`). **Phase 2 (planned):** `tish build` may invoke the same generator automatically so you can skip the separate CLI step. Not supported in the tree-walking interpreter, bytecode VM, or JS output — use **`tish:`** npm native modules there instead. **`tish:polars`** is available when the embedder registers [`tish-polars`](https://github.com/tishlang/tish-polars) via `Evaluator::with_modules` (for example the `tish-polars-run` binary); it exports `Polars` like `import { Polars } from 'tish:polars'`. **`@tishlang/waterui`** (and **`tish:waterui`**) register the same native module when the embedder includes [`@tishlang/waterui`](https://github.com/tishlang/tish-waterui) / `tish-waterui-run`; use `import { version } from '@tishlang/waterui'` or `from 'tish:waterui'`. **`tish:winint`** is registered by [`tish-shadertoy-run`](https://github.com/tishlang/tish-shadertoy) alongside **`tish:shadertoy`**; it currently exports `version` (the local `tish-winint` workspace crate). Like other native crates, this applies to the **Rust native backend** / host binaries, not Cranelift-only or JS output alone.

**`tish:shadertoy`** ([`tish-shadertoy`](https://github.com/tishlang/tish-shadertoy)): `import { … } from 'tish:shadertoy'`. **`run(source)`** opens the viewer and **blocks** until the window closes. For **script-driven** control (native compile path and `tish-shadertoy-run`), use **`openPumpable(source, options?)`** then **`while (pump()) { … }`**: each **`pump()`** returns **`true`** until the user closes the window, so Tish can interleave work (timers, queues, future HTTP hooks) between frames. **`setWindowTitle(title)`** updates the winit title when the window exists. **`reloadShader(source)`** recompiles the fragment shader on the GL thread (for live reload). Optional **`options.onKeyDown`** is a **function** invoked on key down with **`{ key: string, repeat: bool }`**; it works when that function is a **compiled** `Value::Function` (Rust closure from **`tish build`**). The tree-walking runner **`tish-shadertoy-run`** cannot call AST `fn` bodies from winit yet—omit **`onKeyDown`** there or only use **`setWindowTitle` / `reloadShader`** from the **`while (pump())`** body. For **`tish build`**, add the crate as a dependency and set **`package.json` → `tish.module`** (see that repo); the export object is **`shadertoy_object()`** in Rust.

**Optional types (parsed, not enforced):** `let x: number = 1`, `fn f(a: T): R`, `T[]`, `{ k: T }`, `T | U`, rest `...args: T[]`. Function types `(T) => R` parsed for future use.

---

## Semantics

- **Lexical scope** for `let`/`const`: block bodies (`{ … }`), `if`/`else`/`loop` bodies, `catch`, and function bodies each introduce bindings; an inner name can **shadow** an outer one, and the outer binding is visible again after the inner scope ends (see `tests/core/scopes.tish`). **Closures** close over lexical environments as usual.
- **Not** the full ECMAScript binding rules: there is **no `var`**, so no `var`-style hoisting to the whole function/script. Tish also does **not** implement ECMAScript’s **`let`/`const` temporal dead zone** (the spec phase where a binding exists but must not be read before its initializer). Tish uses a **source-order, lexical-frame** model instead: think “declaration runs where it appears,” not “hoisted to block top + TDZ.” Implementations may report errors for invalid use-before-declaration without matching JS TDZ wording.
- `const` cannot be reassigned (runtime error).
- `void expr` evaluates `expr` and returns `null`.
- **`new`:** Parsed like JavaScript (`new` chains, optional `(...)` with spread). On **VM**, **interpreter**, and **native Rust** output, construction uses host **`construct`** / `__construct` (not full ECMA `[[Construct]]`: no `this`, no prototypes). **`tish build --target js`** emits the engine’s **`new`**. There is still **no `class`**, so idiomatic Tish rarely uses `new`; see [ecma-alignment.md](ecma-alignment.md) and the [Tish vs JavaScript](https://tishlang.com/docs/language/vs-javascript) site page for limitations.

---

## Builtins (core)

- **Console:** `log`, `info`, `debug`, `warn`, `error`. Filter with env **`TISH_LOG_LEVEL`**: `debug` | `info` | `log` (default) | `warn` | `error`.
- **Math:** `abs`, `sqrt`, `min`, `max`, `floor`, `ceil`, `round`, …
- **JSON:** `parse`, `stringify`
- **URI:** `encodeURI`, `decodeURI`
- **Parsing:** `parseInt`, `parseFloat`, `isFinite`, `isNaN`
- **Globals:** `Infinity`, `NaN`
- **Number:** `n.toFixed(digits?)` → string (0–20 digits)
- **Object:** `keys`, `values`, `entries`
- **Array:** usual methods (`map`, `filter`, `reduce`, `slice`, `push`, `pop`, …)
- **String (instance):** `length`; `indexOf` / `lastIndexOf` (optional second index; character positions, see note); `includes`; `slice`; `substring`; `split`; `trim`; `toUpperCase` / `toLowerCase`; `startsWith` / `endsWith`; `replace` / `replaceAll`; `charAt` / `charCodeAt`; `repeat`; `padStart` / `padEnd`
- **String (global):** `String.fromCharCode(…)`
- **Note:** String indices follow **Unicode scalar values** (Rust `char`), matching **BMP** JavaScript string indices. **Astral symbols** (e.g. some emoji) are one Tish index but **two UTF-16 code units** in JS, so indices can differ from V8/Node for those characters.

---

## HTTP (`import { … } from 'http'`)

Requires **`http` feature**.

- **`fetch(url, opts?)`** → **`Promise`** → response object: **`status`**, **`ok`**, **`headers`**, **`body`**, **`text`**, **`json`**.
- **`body`** (client response): opaque **`ReadableStream`**, **not** a string. Use **`body.getReader()`** and loop **`await reader.read()`** → **`{ done, value: number[] }`** (raw UTF-8 bytes). Or consume the whole body with **`await res.text()`** or **`await res.json()`** (each returns a **`Promise`**).
- **Single-consumer rule:** after **`getReader()`**, do not use **`await res.text()`** / **`await res.json()`** on the same response (body is locked / consumed).
- **`fetchAll(requests[])`** → **`Promise`** array of the same response shape.
- **`serve(port, handler)`** — server **`req.body`** / response **`body`** are **strings** (not the client stream shape).

**Top-level `await`:** interpreter `tish run …` (module programs). **Native compile:** `async fn main()` + `await` inside.

**See also:** [`tish_runtime/tests/fetch_readable_stream.rs`](https://github.com/tishlang/tish/blob/main/crates/tish_runtime/tests/fetch_readable_stream.rs) (chunked client body over `getReader()`).

---

## Feature flags

| Flag | Enables |
|------|---------|
| `http` | `fetch`, `fetchAll`, `serve`, `Promise`, timers |
| `fs` | `readFile`, `writeFile`, `readDir`, `mkdir` |
| `process` | `process.exit`, `env`, `argv` |
| `regex` | `RegExp` |
| `full` | all of the above |

Build: `cargo build --features full`. CLI artifact output: `tish build … --feature http` (etc.).

---

## Native compile (implementation status)

**Runtime model today:** Values are **dynamically tagged** (`Value` in Rust). Optional type annotations are **parsed only** — they do not yet drive codegen or checked types.

| Route | What you get |
|-------|----------------|
| `tish build --native-backend rust` (default) | Rust source emitted by `tishlang_compile` that calls **`tishlang_runtime`** (`get_index`, `set_index`, arithmetic on `Value`, etc.). `cargo build --release` optimizes the **glue**, not “the Tish program as a flat `f64` kernel.” |
| `tish build --native-backend cranelift` | Native binary that loads **embedded serialized bytecode** and runs **`tishlang_vm`** (`tish_cranelift_runtime`). Cranelift is used only to build a tiny object file holding the blob; **bytecode is not lowered to CLIF**. Throughput is **VM-class** (similar order to `tish run --backend vm`), not “rustc/LLVM on numeric loops.” |
| `tish build --native-backend llvm` | Same **embedded bytecode + VM** link pattern as Cranelift (see `tishlang_llvm` + `tishlang_cranelift_runtime`). |
| `tish build --target js` | Emitted JavaScript; the host (V8, etc.) may **JIT** tight loops. |

**Interop:** `tish:*` and npm-style native imports require **`--native-backend rust`**. The Cranelift/LLVM native-binary paths are **pure Tish** only (no external native modules).

**Direction (in progress):** Where semantics allow, lower to **Rust or machine primitives** (e.g. `Vec<f64>`, `f32`/`f64` buffers, fixed layouts) instead of universal `Value`; use optional types and **inference** to choose representations; add **real bytecode → Cranelift IR** (or similar) for AOT hot paths. The syntax resembles JS/TS; **compiled output is not intended to stay a boxed dynamic VM forever.**

---

## Native UI hooks (`tishlang_ui`, `tish:macos`, `tish:native-ui`, …)

Embeds that ship **`tishlang_ui`** expose **`useState`** and **`useMemo`** on the native module object (e.g. `import { useState, useMemo } from "tish:macos"`).

- **`useState(initial)`** returns a two-element array `[value, setValue]`; **`setValue`** schedules a coalesced re-render (hook cursor resets each pass).
- **`useMemo(factory, deps?)`** runs **`factory()`** once per render pass and **reuses the last result** when **`deps`** is unchanged. Dependencies are compared with a **shallow structural** rule on **`number` / `string` / `bool` / `null` / nested arrays** of those scalars. Omit **`deps`** or pass **`[]`** to memoize for the lifetime of the root. **Function** values are not compared by identity in **`deps`** today.

**`tish:macos` (AppKit):** **`macos.run(App, options?)`** starts the app; the **first** committed root vnode picks the window shell — default content window, or **`sidebar_window`** / **`SidebarWindow`** for **`NSSplitViewController`** (collapsible sidebar) plus a unified toolbar. The sidebar root must have **exactly two** pane subtrees (first = sidebar, second = detail); whitespace-only JSX text between them is ignored. **`macos.openWindow(App, options?)`** opens **another** **`NSWindow`** in the **same process** (a new independent Tish root). **`macos.spawnPeer()`** starts a **second process** (same binary, separate `NSApplication`). **`postSessionMessage`** / **`onSessionMessage`** coordinate peers via distributed notifications and **`TISH_MACOS_SESSION_ID`**.

**Handles and globals:** With **`autoRunEventLoop: false`**, **`macos.run`** returns **`{ show, runEventLoop, spawnPeer, nsWindow }`**. **`macos.openWindow`** returns the same shape. **`nsWindow`** exposes per-window methods (e.g. **`setTitle`**, **`focus`**) for that handle’s **`NSWindow`**. **`app.runEventLoop`**, **`app.spawnPeer`**, and **`app.activate`** are application-wide. Import **`window`** for global **`window.*`** — it resolves to the **current** Tish root’s window (the tree that is rendering or whose UI fired the callback). On **`sidebar_window`**, **`window.innerWidth`** / **`innerHeight`** follow the **detail** pane when the host wires metrics that way.

**`Window` / `SidebarWindow` lifecycle (vnode props):** optional function props **`onOpen`** (alias **`on_open`**), **`onClose`** (**`on_close`**), **`onMinimize`** (**`on_minimize`**), **`onMaximize`** (**`on_maximize`**). **`onOpen`** runs after the window is ordered on-screen; **`onClose`** runs when the window is about to close, before the Tish root is torn down; **`onMinimize`** runs when the window miniaturizes to the Dock; **`onMaximize`** runs when the window becomes zoomed (green traffic-light maximize). Only **compiled** function values are invoked.

**`SidebarWindow` toolbar chrome (vnode props):** the expand/collapse control is AppKit’s **`NSToolbarToggleSidebarItemIdentifier`**. Optional props (default **`true`**): **`sidebarToolbarToggle`** (aliases **`sidebar_toggle`**, **`showSidebarToolbarToggle`**) and **`sidebarTrackingSeparator`** (aliases **`sidebar_tracking_separator`**, **`showSidebarTrackingSeparator`**).

For **`image`**, set **`symbol`** (or **`sfSymbol`** / **`sf_symbol`**) for **`NSImage.imageWithSystemSymbolName`** (SF Symbols); otherwise **`src`** is still a named image or file path as before.

**Render model:** Each flush still re-runs the root component and passes a new vnode tree to the host. **`useMemo`** avoids recomputing **subtrees or derived values** and returns a **`Value`** that can **`Rc`‑reuse** inner vnode objects when unchanged. **Hosts** (e.g. AppKit) may still **rebuild native widgets from scratch** until they implement incremental **`commit_root`** diffing; **`React.memo`‑style automatic component skipping** is not the default in the language today.

---

## CLI

```bash
tish run main.tish
echo 'console.log(1)' | tish run -   # stdin (like `node -`)
echo 'console.log(1)' | tish         # stdin when piped (like `bun`)
echo 'console.log(1)' | tish -      # same; `-` before clap (not a subcommand)
tish build main.tish -o app
tish build main.tish -o app --native-backend cranelift
tish build main.tish -o app --target wasm | wasi | js
```

---

## Informal grammar

```
Program     := Statement*
Statement   := Block | VarDecl | ExprStmt | If | While | For | Return | Break | Continue | FunDecl | Import | …
Block       := Indent Statement* Dedent | '{' Statement* '}'
VarDecl     := ('let'|'const') Ident TypeAnn? ('=' Expr)? ';'?
FunDecl     := ('async')? ('fn'|'function') Ident '(' TypedParams? ')' TypeAnn? ('=' Expr | Block)
For         := 'for' '(' init ';' cond ';' update ')' Stmt
            |  'for' '(' ('let'|'const') Ident 'of' Expr ')' Stmt
TypeAnn     := ':' Type
Type        := Ident | Type '[]' | '{' … '}' | Type '|' Type | '(' … ')' '=>' Type
Expr        := … | NewExpression
NewExpression := 'new' NewExpression | MemberExprNoCall ('(' CallArgs? ')')?
```

---

## Examples

```tish
let name = "World"
console.log(`Hello, ${name}!`)
fn add(a, b) = a + b

import { serve } from 'http'
fn handleRequest(req)
    if req.path === "/health"
        return { status: 200, body: "OK" }
    return { status: 404, body: "Not Found" }
serve(8080, handleRequest)
```

---

## Omitted vs typical JS

No `==`, `var`, `this`, `class`, prototypes, `instanceof`, `delete`, `for..in`, generators, `Symbol`, `BigInt`, `Map`, `Set` (as in spec); prefer Tish docs and tests under `examples/` and `tests/` for edge cases.

**VM note:** The default bytecode VM applies peephole jump chaining. An implementation bug (fixed) once followed `JumpIfFalse` like an unconditional `Jump`, which miscompiled `===` with `||` when nested as an outer `if` condition. See [ecma-alignment.md — Bytecode VM: jump peephole](ecma-alignment.md#bytecode-vm-jump-peephole-implementation).
