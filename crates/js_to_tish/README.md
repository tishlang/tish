# js_to_tish

Vanilla JavaScript to Tish AST converter. Parses JavaScript with OXC, runs semantic analysis, normalizes to Tish's model, and emits `tish_ast::Program`.

## Usage

```rust
use js_to_tish;
use tish_compile_js;

let program = js_to_tish::convert(js_source)?;
let js = tish_compile_js::compile(&program)?;  // output as JavaScript
```

## Output options

- **JavaScript**: `tish_compile_js::compile(&program)` → `String`
- **Bytecode (run)**: `tish_bytecode::compile(&program)` → `Chunk`, then `tish_vm::run(&chunk)`
