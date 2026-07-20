// #203: the `**` operator (BinOp::Pow) inside a hot loop lowers to a host call (== VM's powf), so
// exponentiation kernels JIT instead of bailing. Byte-identical across all backends + node. Covers
// integer/fractional exponents, negative base, 0/negative exponents, and the JS pow edge cases.
function poly(n) {
  let s = 0
  for (let i = 0; i < n; i++) { s = s + i ** 2 - (i + 1) ** 3 + i ** 0.5 }
  return Math.floor(s)
}
console.log("poly", poly(200))
function f(x, y) { return x ** y }
console.log("edges", f(2, 10), f(2, 0.5), f(-8, 3), f(9, -1), f(2, 0), f(0, 0))
console.log("nan", 2 ** (0 / 0), (0 / 0) ** 0, (-1) ** 0.5)
console.log("chain", 2 ** 3 ** 2)
