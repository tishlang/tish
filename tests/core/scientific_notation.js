// Scientific / exponent notation in numeric literals: `e`/`E`, optional sign, digits.
// Lexer-level feature; values are plain f64 so every backend + node agree. (Values are
// kept inside JS's plain-decimal display range — exponent in [-6, 20] — since matching
// V8's exponential `Number.toString` for very large/small numbers is a separate concern.)
console.log(1.5e-3)
console.log(1e10)
console.log(2E+3)
console.log(5e-1)
console.log(1.5e3 + 1)
console.log(3e0)
console.log(1.23e6)
console.log(1e-4)
console.log(10e2)
let x = 2.5e2
console.log(x * 2)
console.log(1.25E2 - 25)
