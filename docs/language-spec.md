# Tish Language Specification

## Overview

Tish is a minimal, TS/JS-compatible language designed for both interpretation and native compilation. Same source runs identically in either backend.

## Syntax Summary

### Keywords

- `fun` — function declaration (replaces `function`)
- `any` — variable declaration (replaces `let`; block-scoped)
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
- `for (any x of arr)` — iterate arrays and strings
- `switch (expr) { case val: stmt... default: stmt }`
- `break`, `continue`, `return expr`
- `throw expr` / `try stmt catch (e) stmt`
- `typeof expr` — returns `"number"`, `"string"`, `"boolean"`, `"object"`, `"function"`
- `void expr` — evaluates expr, returns `null` (Tish uses null instead of JS undefined)
- Postfix `++` / `--` on identifiers

Blocks: `{ stmt; stmt }` or indentation (Indent/Dedent tokens).

### Functions

```tish
fun name(a, b) { return a + b }
fun double(x) = x * 2   // single-expression, implicit return
```

### Builtins

- `print(...)` — print args space-separated
- `parseInt(s, radix?)`, `parseFloat(s)`
- `isFinite(v)`, `isNaN(v)`
- `Infinity`, `NaN` — globals
- `Math.abs(x)`, `Math.sqrt(x)`, `Math.min(a, b, ...)`, `Math.max(a, b, ...)`, `Math.floor(x)`, `Math.ceil(x)`, `Math.round(x)`

### Assignment

`x = expr` — assigns to existing variable (no `const`/`let`).

## Indentation

- Braces optional: use indentation for blocks.
- Tab and space normalized: 1 tab = 1 level; 2 spaces = 1 level.
- No mixing errors: both styles work; only consistent level matters.

## Semantics

- **Block scope**: Variables declared with `any` are block-scoped. No hoisting.
- **Strict equality only**: `===` / `!==`; no loose coercion.
- **No `this`**: Use explicit parameters.
- **No prototypes**: Plain objects and arrays; fixed shapes.
- **Closures**: Functions capture by name; lexical scope.

## Grammar (informal)

```
Program     := Statement*
Statement   := Block | VarDecl | ExprStmt | If | While | For | Return | Break | Continue | FunDecl
Block       := Indent Statement* Dedent  |  '{' Statement* '}'
VarDecl     := 'any' Ident ('=' Expr)? ';'?
ExprStmt    := Expr ';'?
If          := 'if' '(' Expr ')' Statement ('else' Statement)?
While       := 'while' '(' Expr ')' Statement
For         := 'for' '(' Init? ';' Cond? ';' Update? ')' Statement  |  'for' '(' 'any' Ident 'of' Expr ')' Statement
Return      := 'return' Expr? ';'?
FunDecl     := 'fun' Ident '(' Params? ')' '=' Expr  |  'fun' Ident '(' Params? ')' Block
Expr        := Assign | NullishCoalesce | Or | ...
Assign      := Ident '=' Expr
NullishCoalesce := Or ('??' Or)*
```
