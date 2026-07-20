// #203: Math.<CONST> inside a function lowers to a LoadConst when Math is the unshadowed global, so
// constant-using numeric kernels JIT (no GetMember bail). Values match the Math object on every
// backend (#539), so byte-identical to node. A shadowed Math must read the user's value.
function geo(n) {
  let s = 0
  for (let i = 0; i < n; i++) { s = s + i * Math.PI + Math.E - Math.SQRT2 * i + Math.LN2 }
  return Math.floor(s * 1000)
}
console.log("geo", geo(100))
console.log("consts", Math.PI, Math.E, Math.LN2, Math.LN10, Math.LOG2E, Math.LOG10E, Math.SQRT2, Math.SQRT1_2)
function usesConst(x) { return x * Math.PI + Math.E }
console.log("fn", usesConst(2))
// SOUNDNESS: a shadowed Math.PI must read the user's object, not the intrinsic constant.
function shadowed() { let Math = { PI: 99 }; return Math.PI }
console.log("shadow", shadowed())
