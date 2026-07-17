// #428: `async` in EXPRESSION positions parses on every backend — async arrows (paren and
// single-param, #436) and async function EXPRESSIONS (anonymous, named, object-property, IIFE
// operand — this PR). Assertions are typeof-only: tish's concurrency model resolves an async
// call synchronously (no microtask queue) while node returns a Promise, so consuming a call
// RESULT here would diverge; the await-consumption path is exercised by vm/native alongside
// node in the PR validation, and the interp's user-async await gap is tracked separately.

let add = async function(a, b) { return a + b }
console.log(typeof add)

let named = async function fx(n) { return n * 2 }
console.log(typeof named)

let o = { m: async function() { return 3 } }
console.log(typeof o.m)

console.log(typeof (async function() { return 9 }))

let ar = async () => 1
let ar1 = async x => x
console.log(typeof ar, typeof ar1)
