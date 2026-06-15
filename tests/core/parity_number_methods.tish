// Cross-backend parity: member access / method calls on numeric literals. `(255).toString(16)` must
// compile to valid JS — emitting `255.toString(16)` is a JS syntax error (the lexer reads `255.` as
// a float). The parens must be preserved for integer literals (and folded integer constants). Every
// backend (interp/vm/native/cranelift/wasi/js) must agree with node. Valid in tish and node.

console.log("hex", (255).toString(16))
console.log("bin", (255).toString(2))
console.log("dec", (10).toString())
console.log("fixed", (3.14159).toFixed(2))
console.log("fixed1", (10).toFixed(1))
console.log("neg", (-5).toString())
console.log("folded-bin", (100 * 2).toString(2))
console.log("folded-hex", (16 * 16 - 1).toString(16))
console.log("big", (1000000).toString())
console.log("chained", (255).toString(16).toUpperCase())
