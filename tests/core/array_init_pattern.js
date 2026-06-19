// #173: array fill-loop fusion, native `.length`, and OOB-growing index stores.
// Valid in both tish and node (no type annotations). Deterministic integer checks so the
// typed-native fast paths (fused fill, native length) stay byte-identical to the boxed path & node.

// (1) Positive fill loop over a boolean[] with a literal bound -> fuses to one bulk fill.
function boolFill() {
  let a = []
  for (let i = 0; i < 8; i++) { a.push(true) }
  let trues = 0
  for (let i = 0; i < a.length; i++) { if (a[i]) { trues = trues + 1 } }
  return a.length * 100 + trues
}

// (2) Positive fill over a number[] with an integer-`let` bound, then in-bounds sieve writes.
function numFillSieve() {
  let n = 30
  let isPrime = []
  for (let i = 0; i < n; i++) { isPrime.push(1) }
  let count = 0
  for (let i = 2; i < n; i++) {
    if (isPrime[i] === 1) {
      count = count + 1
      let k = i + i
      while (k < n) { isPrime[k] = 0; k = k + i }
    }
  }
  return count
}

// (3) `.length` before/after pushes (native length on a Vec).
function lengthProbe() {
  let a = []
  let before = a.length
  for (let i = 0; i < 5; i++) { a.push(i) }
  let mid = a.length
  a.push(99)
  return before * 1000 + mid * 10 + a.length
}

// (4) OOB store past the end grows the array; the holes read back falsy.
function oobGrow() {
  let a = []
  for (let i = 0; i < 3; i++) { a.push(1) }
  a[6] = 9
  let holeFalsy = a[4] ? 1 : 0
  return a.length * 10 + holeFalsy
}

// (5) Adversarial: non-constant push arg -> must NOT fuse, stays correct.
function nonConstFill() {
  let a = []
  for (let i = 0; i < 6; i++) { a.push(i * 2) }
  let s = 0
  for (let i = 0; i < a.length; i++) { s = s + a[i] }
  return s
}

// (6) Adversarial: extra statement in loop body -> no fusion.
function extraStmt() {
  let a = []
  let t = 0
  for (let i = 0; i < 7; i++) { a.push(true); t = t + 1 }
  return a.length * 10 + t
}

// (7) Adversarial: break in the loop -> no fusion.
function withBreak() {
  let a = []
  for (let i = 0; i < 10; i++) { if (i === 4) { break } a.push(true) }
  return a.length
}

console.log("boolFill " + boolFill())
console.log("sieve " + numFillSieve())
console.log("length " + lengthProbe())
console.log("oob " + oobGrow())
console.log("nonconst " + nonConstFill())
console.log("extra " + extraStmt())
console.log("break " + withBreak())
