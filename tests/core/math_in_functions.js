// #186/#203: Math.<fn> called INSIDE a function must (a) compute the correct result and (b) when
// `Math` is provably the unshadowed global, lower to the MathUnary intrinsic so numeric/array
// kernels JIT — while a SHADOWED `Math` (local, param, or captured) must still call the user's value.
// Byte-identical across interp/vm/native/cranelift/wasi/js/node. Locks the compiler's math_is_global
// threading into nested-function compilers.

// Math.* inside a plain function (the intrinsic path once math_is_global threads).
function kernel(n) {
  let s = 0
  for (let i = 0; i < n; i++) {
    s = s + Math.floor(Math.sqrt(i) * 2) + Math.abs(Math.round(i / 3)) - Math.trunc(i / 7)
  }
  return s
}
console.log("kernel", kernel(50))

// Nested function using Math.
function outer(n) {
  function inner(x) { return Math.ceil(Math.sqrt(x)) }
  let t = 0
  for (let i = 0; i < n; i++) { t = t + inner(i) }
  return t
}
console.log("nested", outer(30))

// Trig / transcendental (host-call MathUnary path).
function trig(n) {
  let a = 0
  for (let i = 0; i < n; i++) { a = a + Math.sin(i) * Math.cos(i) + Math.exp(i / 100) }
  return Math.floor(a * 1000)
}
console.log("trig", trig(20))

// Math at top level (already worked) still correct.
console.log("toplevel", Math.floor(3.9), Math.max(1, 2, 3), Math.min(4, 5))
