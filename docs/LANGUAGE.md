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

**Modules:** `import { a } from 'http'`; native `tish:fs`, `tish:http`, `tish:process`, `tish:ws`, etc. (Rust backend only). **`tish:polars`** is available when the embedder registers [`tish-polars`](https://github.com/tishlang/tish-polars) via `Evaluator::with_modules` (for example the `tish-polars-run` binary); it exports `Polars` like `import { Polars } from 'tish:polars'`.

**Optional types (parsed, not enforced):** `let x: number = 1`, `fn f(a: T): R`, `T[]`, `{ k: T }`, `T | U`, rest `...args: T[]`. Function types `(T) => R` parsed for future use.

---

## Semantics

- Block scope for `let`/`const`; no hoisting of declarations like JS `var`.
- `const` cannot be reassigned (runtime error).
- Closures; lexical scope.
- `void expr` evaluates `expr` and returns `null`.
- **`new`:** Parsed like JavaScript (`new` chains, optional `(...)` with spread). On **VM**, **interpreter**, and **native Rust** output, construction uses host **`construct`** / `__construct` (not full ECMA `[[Construct]]`: no `this`, no prototypes). **`tish compile --target js`** emits the engine’s **`new`**. There is still **no `class`**, so idiomatic Tish rarely uses `new`; see [ecma-alignment.md](ecma-alignment.md) and the [Tish vs JavaScript](https://tishlang.com/docs/language/vs-javascript) site page for limitations.

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
- **Array / string:** usual methods (`map`, `filter`, `reduce`, `slice`, `split`, …)

---

## HTTP (`import { … } from 'http'`)

Requires **`http` feature**. **`fetch(url, opts?)`** → Promise → response `{ status, ok, headers, body, text, json }`. Body: stream **or** `text`/`json` — **one consumer** (second use throws). **`fetchAll(requests[])`**, **`serve(port, handler)`**.

**Top-level `await`:** interpreter `tish run … --backend interp`. **Native compile:** `async fn main()` + `await` inside.

---

## Feature flags

| Flag | Enables |
|------|---------|
| `http` | `fetch`, `fetchAll`, `serve`, `Promise`, timers |
| `fs` | `readFile`, `writeFile`, `readDir`, `mkdir` |
| `process` | `process.exit`, `env`, `argv` |
| `regex` | `RegExp` |
| `full` | all of the above |

Build: `cargo build --features full`. Compile: `tish compile … --feature http` (etc.).

---

## Native compile (implementation status)

**Runtime model today:** Values are **dynamically tagged** (`Value` in Rust). Optional type annotations are **parsed only** — they do not yet drive codegen or checked types.

| Route | What you get |
|-------|----------------|
| `tish compile --native-backend rust` (default) | Rust source emitted by `tishlang_compile` that calls **`tishlang_runtime`** (`get_index`, `set_index`, arithmetic on `Value`, etc.). `cargo build --release` optimizes the **glue**, not “the Tish program as a flat `f64` kernel.” |
| `tish compile --native-backend cranelift` | Native binary that loads **embedded serialized bytecode** and runs **`tishlang_vm`** (`tish_cranelift_runtime`). Cranelift is used only to build a tiny object file holding the blob; **bytecode is not lowered to CLIF**. Throughput is **VM-class** (similar order to `tish run --backend vm`), not “rustc/LLVM on numeric loops.” |
| `tish compile --native-backend llvm` | Same **embedded bytecode + VM** link pattern as Cranelift (see `tishlang_llvm` + `tishlang_cranelift_runtime`). |
| `tish compile --target js` | Emitted JavaScript; the host (V8, etc.) may **JIT** tight loops. |

**Interop:** `tish:*` and npm-style native imports require **`--native-backend rust`**. The Cranelift/LLVM native-binary paths are **pure Tish** only (no external native modules).

**Direction (in progress):** Where semantics allow, lower to **Rust or machine primitives** (e.g. `Vec<f64>`, `f32`/`f64` buffers, fixed layouts) instead of universal `Value`; use optional types and **inference** to choose representations; add **real bytecode → Cranelift IR** (or similar) for AOT hot paths. The syntax resembles JS/TS; **compiled output is not intended to stay a boxed dynamic VM forever.**

---

## CLI

```bash
tish run main.tish
echo 'console.log(1)' | tish run -   # stdin (like `node -`)
echo 'console.log(1)' | tish         # stdin when piped (like `bun`)
echo 'console.log(1)' | tish -      # same; `-` before clap (not a subcommand)
tish compile main.tish -o app
tish compile main.tish -o app --native-backend cranelift
tish compile main.tish -o app --target wasm | wasi | js
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
