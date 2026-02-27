# Tish

Minimal, TS/JS-compatible language. Runs via interpreter or compiles to native. See `docs/` for spec and ECMA alignment.

## Quick Example

```javascript
// hello.tish
let name = "World"
console.log(`Hello, ${name}!`)

fun add(a, b) = a + b
console.log(`1 + 2 = ${add(1, 2)}`)
```

```bash
tish run hello.tish
# Hello, World!
# 1 + 2 = 3
```

**Log levels**: Control output with `TISH_LOG_LEVEL=debug|info|log|warn|error`

## Build

```bash
cargo build --release -p tish
```

The binary is `target/release/tish`. Add it to your PATH or run directly.

## Run (Interpreter)

```bash
tish run <file.tish>
```

## Compile to Native Binary

```bash
cargo run -p tish -- compile <file.tish> -o <output>
./<output>
```

This generates a standalone native executable. Requires `rustc` and must be run from the workspace root (needs access to `crates/tish_runtime`).

Example:
```bash
cargo run -p tish -- compile hello.tish -o hello
./hello
# Hello, World!
# 1 + 2 = 3
```

**Note**: The compiled binary is fully standalone and can be distributed without Tish or Rust.

## Test

```bash
cargo test -p tish
```

Tests:
- `test_full_stack_parse` – lex + parse each .tish file
- `test_mvp_programs_interpreter` – run via interpreter
- `test_mvp_programs_interpreter_vs_native` – compile to native, compare output

Run any test file: `tish run tests/mvp/<name>.tish`

## Performance Comparison

JavaScript equivalents in `performance/mvp/*.js`. Compare Tish vs Node.js/Bun:

```bash
./scripts/run_performance_manual.sh
```

## Features

- **Variables**: `let` (mutable), `const` (immutable)
- **Functions**: `fun name(a, b) { ... }` or `fun name(a) = expr`
- **Arrow functions**: `x => x * 2`, `(a, b) => a + b`
- **Template literals**: `` `Hello, ${name}!` ``
- **Control flow**: `if/else`, `while`, `for`, `for..of`, `switch`
- **Operators**: `+`, `-`, `*`, `/`, `%`, `**`, `===`, `!==`, `&&`, `||`, `??`, `?.`
- **Data**: Arrays `[]`, Objects `{}`, with mutation support
- **Built-ins**: `console.log`, `Math.*`, `JSON.*`, `Object.keys/values/entries`
- **Array methods**: `map`, `filter`, `reduce`, `find`, `forEach`, `push`, `pop`, etc.
- **String methods**: `slice`, `split`, `trim`, `toUpperCase`, `includes`, etc.

See `docs/plan-gap-analysis.md` for full feature list and JS compatibility.
