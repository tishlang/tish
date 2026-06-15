// #247 feature gaps now implemented across interp/vm/native/cranelift/wasi/js: Array.findLast/
// findLastIndex, Array.at, String.at (negative index from end; out of range -> null). Valid in
// tish and node (node's `undefined` for OOB normalizes to tish's `null` in the parity harness).
let a = [5, 12, 8, 20, 3]
console.log("findLast-even", a.findLast(x => x % 2 === 0))
console.log("findLastIndex-even", a.findLastIndex(x => x % 2 === 0))
console.log("findLast-none", a.findLast(x => x > 99))
console.log("at0", a.at(0))
console.log("at-neg1", a.at(-1))
console.log("at-neg2", a.at(-2))
console.log("at-oob", a.at(99))
let s = "world"
console.log("sat2", s.at(2))
console.log("sat-neg1", s.at(-1))
console.log("sat-oob", s.at(99))
