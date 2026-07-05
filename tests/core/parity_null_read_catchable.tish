// Reading a property or index of the nullish value throws a CATCHABLE TypeError on every backend +
// node (interp/vm/cranelift/wasi already threw; native now parks a catchable throw — #425). Only
// `e.name` is asserted (messages differ across engines). Catch-based so the parked-throw surfacing is
// observed at the enclosing `try`. Valid in tish and node.

// property read on null (try body contains a call)
let a = "no-throw"
try {
  let z = null
  console.log("unreachable " + z.length)
} catch (e) { a = e.name }
console.log("member-call", a)

// property read on null — PURE read in the try body (no call): the throw must still be caught
let b = "no-throw"
try {
  let z = null
  let x = z.length
  let y = x + 1
} catch (e) { b = e.name }
console.log("member-pure", b)

// index read on null
let c = "no-throw"
try {
  let z = null
  let x = z[0]
} catch (e) { c = e.name }
console.log("index", c)

// program keeps running after each catch
console.log("survived", true)

// valid reads are unaffected
let o = { a: 42, b: 7 }
console.log("valid-prop", o.a + o.b)
let arr = [10, 20, 30]
console.log("valid-index", arr[1])
console.log("valid-len", arr.length)

// an out-of-bounds array index does NOT throw (only a nullish RECEIVER throws)
let e2 = "no-throw"
try {
  let arr2 = [1, 2, 3]
  let x = arr2[9]
} catch (e) { e2 = "threw" }
console.log("oob-index-no-throw", e2)
