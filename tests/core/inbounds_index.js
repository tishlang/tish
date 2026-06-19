// #173 part 3: in-bounds index elision must stay byte-identical to the boxed path and node, and must
// NOT fire when an array could be mutated out of line. Valid in both tish and node; deterministic.

// (1) In-bounds writes/reads under a `i < a.length` guard over a fixed-length array (elided).
function fixedSquares() {
  let a = []
  for (let i = 0; i < 10; i++) { a.push(0) }
  for (let i = 0; i < a.length; i++) { a[i] = i * i }
  let s = 0
  for (let i = 0; i < a.length; i++) { s = s + a[i] }
  return s
}

// (2) Strided in-bounds store guarded by a `while (k < n)` whose counter is reassigned in the body
// AFTER the store — the store is still in-bounds, the value after it isn't (must not be elided).
function strided() {
  let n = 50
  let marks = []
  for (let i = 0; i < n; i++) { marks.push(0) }
  let hits = 0
  for (let i = 1; i < n; i++) {
    let k = i
    while (k < n) { marks[k] = marks[k] + 1; k = k + i }
  }
  for (let i = 0; i < n; i++) { hits = hits + marks[i] }
  return hits
}

// (3) Escape: an array passed to a function could be shrunk out of line, so the fixed-length fact
// must NOT apply — its stores keep the OOB-safe (resize) lowering. Still correct here.
function shrinkIt(b) {
  b.pop()
  return b.length
}
function escaping() {
  let a = []
  for (let i = 0; i < 5; i++) { a.push(7) }
  let lenAfter = shrinkIt(a)
  a[a.length] = 3
  return lenAfter * 100 + a.length
}

console.log("fixed " + fixedSquares())
console.log("strided " + strided())
console.log("escaping " + escaping())
