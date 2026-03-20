# Tish — LLM Quick Reference

> JS/TS-like, multi-target language. Same source runs via **interpreter** or compiles to **native** / **WASM** / **JS**. Built for the JS ecosystem.

<!-- TOC: Identity | Syntax | JS vs Tish | Execution | Examples | Features | Rust | Native Modules -->

---

## 1. Identity

- **Tish** = minimal TS/JS-compatible language
- **Multi-target:** interpreter, native (Rust/Cranelift), WASM (browser/WASI), JS transpile
- **Secure-by-default:** network, filesystem, process are feature-gated
- No `undefined` — use `null`. No `this`, prototypes, classes, or `new`.

---

## 2. Syntax

### Keywords

`fn` / `function` · `let` · `const` · `if` `else` · `while` · `for` · `switch` `case` `default` · `return` `break` `continue` · `try` `catch` `throw` · `async` `await` · `typeof` `void` · `true` `false` `null`

### Literals

Numbers, strings `"`/`'`, arrays `[]`, objects `{}`, template literals `` `Hello, ${name}!` ``

### Operators

- **Equality:** `===` `!==` only (strict; no coercion)
- **Logical:** `&&` `||` `!` · `??` (nullish coalesce) · `?.` (optional chaining)
- **Arithmetic:** `+` `-` `*` `/` `%` `**` · `+=` `-=` etc.
- **Bitwise:** `&` `|` `^` `~` `<<` `>>`

### Functions

```tish
fn add(a, b) { return a + b }
fn double(x) = x * 2          // single-expr, implicit return
let f = (a, b) => a + b       // arrow
async fn fetchData(url) {
    let res = await fetch(url)
    return res.ok ? await res.text() : null
}
```

### Control flow

`if`/`else` · `while` · `do`/`while` · C-style `for` · `for (let x of arr)` · `switch` · `try`/`catch`

### Blocks

Braces `{ }` **or** indentation (tab/space)

### Type annotations (optional)

Parsed, not enforced: `let x: number = 42` · `fn add(a: number, b: number): number`

---

## 3. JS vs Tish

| JS | Tish |
|----|------|
| `undefined` | `null` only |
| `typeof null` → `"object"` | → `"null"` |
| `==` and `===` | `===` / `!==` only |
| `var`, `let`, `const` | `let`, `const` only |
| Braces required | Braces optional (indent) |
| `this` | No `this`; use explicit params |
| Prototypes, `instanceof` | Plain objects only |
| `class`, `new`, `super` | None |
| Optional chaining → `undefined` | → `null` |
| `delete`, `for..in`, generators | Omitted |
| Symbol, BigInt, Map, Set | Omitted |

---

## 4. Execution

```bash
tish run main.tish                          # interpreter
tish compile main.tish -o app               # native (Rust backend)
tish compile main.tish -o app --native-backend cranelift   # pure Tish, no native imports
tish compile main.tish -o app --target wasm # browser WASM
tish compile main.tish -o app --target wasi # Wasmtime/WASI
tish compile main.tish -o app --target js   # JS transpile
```

**Native:** needs `rustc`, Cargo, workspace root. `--native-backend rust` = full ecosystem, supports `tish:*` and `@scope/pkg`. `--native-backend cranelift` = pure Tish, no native imports, faster build, curated subset.

---

## 5. Examples

### Hello + fn

```tish
let name = "World"
console.log(`Hello, ${name}!`)

fn add(a, b) = a + b
console.log(`1 + 2 = ${add(1, 2)}`)
```

### HTTP server

```tish
import { serve } from 'http'

fn handleRequest(req) {
    if (req.path === "/health") return { status: 200, body: "OK" }
    if (req.path === "/") return {
        status: 200,
        headers: { contentType: "application/json" },
        body: JSON.stringify({ message: "Hello" })
    }
    return { status: 404, body: "Not Found" }
}

serve(8080, handleRequest)
```

### Async fetch

```tish
import { fetchAll } from 'http'

let urls = ["https://httpbin.org/get", "https://httpbin.org/uuid"]
let results = await fetchAll(urls.map(u => ({ url: u })))
console.log("ok:", results.every(r => r.ok))
```

---

## 6. Feature flags (secure-by-default)

| Flag | Enables |
|------|---------|
| `http` | `fetch`, `fetchAll`, `serve`; `Promise`; `setTimeout`, `setInterval` |
| `fs` | `readFile`, `writeFile`, `readDir`, `mkdir` |
| `process` | `process.exit`, `process.env`, `process.argv` |
| `regex` | `RegExp` |
| `full` | All above |

- **Run:** build tish with `--features full` (or specific flags)
- **Compile:** `tish compile main.tish -o app --feature http --feature fs`

---

## 7. Native modules (imports)

```tish
import { serve, fetch, fetchAll } from 'http'
import { readFile, writeFile } from 'tish:fs'
import { process } from 'tish:process'
import { Server } from 'tish:ws'
```

Requires `--native-backend rust` (not cranelift). Feature flags apply.

---

## 8. Builtins (always available)

`console.log` `info` `warn` `error` · `Math.*` · `JSON.parse` `JSON.stringify` · `parseInt` `parseFloat` · `Object.keys` `values` `entries` · `encodeURI` `decodeURI` · `Infinity` `NaN` · Array/string methods (`map`, `filter`, `reduce`, `slice`, `split`, etc.)
