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

## Two Ways to Execute Tish Programs

Tish supports **two execution modes**: interpret or compile to native.

### 1. RUN (Interpreter)

Execute `.tish` files directly without a build step:

```bash
tish run hello.tish
# Hello, World!
# 1 + 2 = 3
```

Best for: development, scripting, quick iteration.

### 2. BUILD (Compile to Native)

Compile `.tish` files to standalone native executables:

```bash
tish compile hello.tish -o hello
./hello
# Hello, World!
# 1 + 2 = 3
```

Best for: distribution, performance, deploying without Tish installed.

The compiled binary is **fully standalone** — no Tish or Rust runtime needed to run it.

## Installing Tish

```bash
cargo build --release -p tish
```

The binary is `target/release/tish`. Add it to your PATH or run directly.

**Note**: Compiling to native (`tish compile`) requires `rustc` and must be run from the workspace root (needs access to `crates/tish_runtime`).

## Using just (Recommended)

The project includes a `justfile` for common tasks:

```bash
# Run a tish file (interpreter, all features)
just run run hello.tish

# Compile to native binary
just compile hello.tish hello
./hello

# Run in secure mode (no network/fs/process access)
just run-secure run hello.tish
```

See `just --list` for all available recipes.

## Feature Flags

Tish has compile-time feature flags for security:

| Flag | Enables |
|------|---------|
| `http` | Network access (`fetch`, `fetchAll`, `serve`) |
| `fs` | File system (`readFile`, `writeFile`, `mkdir`, etc.) |
| `process` | Process control (`process.exit`, `process.env`, etc.) |
| `regex` | Regular expressions (`RegExp`, `String.match`, etc.) |
| `full` | All features |

Default: **no features** (secure mode). Use `--features full` for development.

**Log levels**: Control output with `TISH_LOG_LEVEL=debug|info|log|warn|error`

## Test

```bash
cargo test -p tish
```

Tests:
- `test_full_stack_parse` – lex + parse each .tish file
- `test_mvp_programs_interpreter` – run via interpreter
- `test_mvp_programs_interpreter_vs_native` – compile to native, compare output

Run any test file: `tish run tests/core/<name>.tish`

## Performance Comparison

JavaScript equivalents in `tests/core/*.js`. Compare Tish vs Node.js/Bun:

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
