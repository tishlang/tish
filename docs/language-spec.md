# Tish Language Specification

## Overview

Tish is a minimal, TS/JS-compatible language designed for both interpretation and native compilation. Same source runs identically in either backend.

## Syntax Summary

### Keywords

- `fn` — function declaration (replaces `function`; `function` also supported)
- `let` — mutable variable declaration (block-scoped)
- `const` — immutable variable declaration (block-scoped, error on reassignment)
- `if`, `else`, `while`, `for`, `return`, `break`, `continue`, `switch`, `case`, `default`, `do`, `throw`, `try`, `catch`, `typeof`
- `true`, `false`, `null`

### Literals

- Numbers: `1`, `1.5`, `0.5`
- Strings: `"hello"`, `'world'` (escapes: `\n`, `\r`, `\t`, `\\`, `\"`, `\'`) — `.length` returns character count
- Booleans: `true`, `false`
- Null: `null`
- Arrays: `[1, 2, 3]` — `.length` returns element count
- Objects: `{ x: 1, y: 2 }` (fixed keys at parse time)

### Operators

| Op  | Meaning                 |
|-----|-------------------------|
| `+` | Add (numbers) / concat (strings) |
| `-` `*` `/` `%` `**` | Arithmetic (`**` = exponentiation) |
| `&` `|` `^` `~` `<<` `>>` | Bitwise (32-bit integer semantics) |
| `===` `!==` | Strict equality (no coercion) |
| `<` `<=` `>` `>=` | Comparison |
| `&&` `\|\|` `!` | Logical |
| `? :` | Conditional (ternary) |
| `??` | Nullish coalescing |
| `?.` | Optional chaining |

### Control Flow

- `if (cond) stmt` / `if (cond) stmt else stmt`
- `while (cond) stmt` / `do stmt while (cond)`
- `for (init; cond; update) stmt` — C-style
- `for (let x of arr)` — iterate arrays and strings
- `for (const x of arr)` — iterate with immutable binding
- `switch (expr) { case val: stmt... default: stmt }`
- `break`, `continue`, `return expr`
- `throw expr` / `try stmt catch (e) stmt`
- `typeof expr` — returns `"number"`, `"string"`, `"boolean"`, `"null"`, `"object"`, `"function"` (Tish returns `"null"` for null; JS returns `"object"`)
- `void expr` — evaluates expr, returns `null` (Tish uses null instead of JS undefined)
- Postfix `++` / `--` on identifiers

Blocks: `{ stmt; stmt }` or indentation (Indent/Dedent tokens).

### Functions

```tish
fn name(a, b) { return a + b }
fn double(x) = x * 2   // single-expression, implicit return

// Async functions (use with await)
async fn fetchData(url) {
    let res = await fetchAsync(url)
    return res.ok ? res.body : null
}
```

### Builtins

**Console (log levels)**:
- `console.log(...)` — general output (default level)
- `console.info(...)` — informational messages
- `console.debug(...)` — debug messages (hidden by default)
- `console.warn(...)` — warnings (outputs to stderr)
- `console.error(...)` — errors (always outputs to stderr)

Log level controlled via `TISH_LOG_LEVEL` environment variable:
- Values: `debug`, `info`, `log` (default), `warn`, `error`
- Example: `TISH_LOG_LEVEL=debug ./program` shows all messages
- Example: `TISH_LOG_LEVEL=warn ./program` shows only warnings and errors

**Parsing**:
- `parseInt(s, radix?)`, `parseFloat(s)`
- `isFinite(v)`, `isNaN(v)`

**Globals**:
- `Infinity`, `NaN`

**Math**:
- `Math.abs(x)`, `Math.sqrt(x)`, `Math.min(a, b, ...)`, `Math.max(a, b, ...)`, `Math.floor(x)`, `Math.ceil(x)`, `Math.round(x)`

**Number**:
- `n.toFixed(digits?)` — format number with fixed decimal places (0–20), returns string

**JSON**:
- `JSON.parse(s)`, `JSON.stringify(v)`

**URI**:
- `encodeURI(s)`, `decodeURI(s)`

### Assignment

- `x = expr` — assigns to existing `let` variable
- Compound: `x += expr`, `x -= expr`, `x *= expr`, `x /= expr`, `x %= expr`
- **const variables cannot be reassigned** (runtime error)

## Indentation

- Braces optional: use indentation for blocks.
- Tab and space normalized: 1 tab = 1 level; 2 spaces = 1 level.
- No mixing errors: both styles work; only consistent level matters.

## Semantics

- **Block scope**: Variables declared with `let`/`const` are block-scoped. No hoisting.
- **Immutability**: `const` bindings cannot be reassigned (like JavaScript).
- **Strict equality only**: `===` / `!==`; no loose coercion.
- **No `this`**: Use explicit parameters.
- **No prototypes**: Plain objects and arrays; fixed shapes.
- **Closures**: Functions capture by name; lexical scope.

## Type Annotations (Optional)

Tish supports optional TypeScript-style type annotations. Types are parsed but not enforced at runtime (gradual typing).

### Syntax

```tish
// Variable declarations
let x: number = 42
const name: string = "hello"
let arr: number[] = [1, 2, 3]

// Function parameters and return types
fn add(a: number, b: number): number {
    return a + b
}

// Object types
let person: { name: string, age: number } = { name: "Alice", age: 30 }

// Union types
let value: number | string = 42

// Rest parameters
fn sum(...args: number[]): number { ... }
```

### Supported Types

| Type | Description |
|------|-------------|
| `number` | Numeric values (f64) |
| `string` | String values |
| `boolean` | `true` or `false` |
| `null` | The null value |
| `void` | No return value (functions) |
| `T[]` | Array of type T |
| `{ k: T }` | Object with typed properties |
| `T \| U` | Union (either T or U) |
| `(T) => R` | Function type (future) |

### Notes

- Type annotations are optional; omitting them is equivalent to dynamic typing
- Types are parsed and stored in the AST but not enforced during evaluation (Phase 2)
- Future phases will add type inference and type checking

## Grammar (informal)

```
Program     := Statement*
Statement   := Block | VarDecl | ExprStmt | If | While | For | Return | Break | Continue | FunDecl
Block       := Indent Statement* Dedent  |  '{' Statement* '}'
VarDecl     := ('let' | 'const') Ident TypeAnn? ('=' Expr)? ';'?
ExprStmt    := Expr ';'?
If          := 'if' '(' Expr ')' Statement ('else' Statement)?
While       := 'while' '(' Expr ')' Statement
For         := 'for' '(' Init? ';' Cond? ';' Update? ')' Statement  |  'for' '(' ('let'|'const') Ident 'of' Expr ')' Statement
Return      := 'return' Expr? ';'?
FunDecl     := ('fn' | 'function') Ident '(' TypedParams? ')' TypeAnn? '=' Expr  |  ('fn' | 'function') Ident '(' TypedParams? ')' TypeAnn? Block
Expr        := Assign | NullishCoalesce | Or | ...
Assign      := Ident '=' Expr
NullishCoalesce := Or ('??' Or)*

TypeAnn     := ':' Type
Type        := Ident | Type '[]' | '{' (Ident TypeAnn ',')* '}' | '(' (Type ',')* ')' '=>' Type | Type '|' Type
TypedParams := TypedParam (',' TypedParam)*
TypedParam  := Ident TypeAnn?
```
