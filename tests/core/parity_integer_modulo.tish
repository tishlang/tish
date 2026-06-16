// Integer-modulo lattice (#174): `x % c` where `x` is a proven integer in (-2^53, 2^53) and `c` is
// a positive integer literal lowers (native typed backend) to an `i64` remainder instead of `fmod`.
// Must stay bit-identical to interp/vm/cranelift/wasi/node. Covers: a `% c`-bounded LCG recurrence,
// nested modulo, NEGATIVE dividends (sign follows dividend, like JS/Rust truncation), and a value
// just under 2^53. `r % 97` uses a literal-bounded loop counter as the dividend.
function lcg(n) {
  let seed = 42
  let sum = 0
  for (let i = 0; i < n; i++) {
    seed = (seed * 16807 + 12345) % 2147483647
    sum = (sum + (seed % 100)) % 1000000007
  }
  return sum
}
function negmod(n) {
  let x = -1000
  let acc = 0
  for (let i = 0; i < n; i++) { x = (x - 7) % 13; acc = (acc + x) | 0 }
  return acc
}
function countermod(n) { let s = 0; for (let i = 0; i < n; i++) { s = (s + (i % 97)) % 100003 } return s }
console.log("lcg", lcg(50000))
console.log("negmod", negmod(500))
console.log("countermod", countermod(100000))
console.log("near2p53", (9007199254740990 % 1000000))
console.log("negsmall", ((-7) % 3))
console.log("negbig", ((-123456789) % 1000))
