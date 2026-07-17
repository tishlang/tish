// Node oracle for parity_nested_closure_capture.tish (#467).
let base = 10

let f = function(x) { return function(y) { return base + x + y } }
console.log(f(1)(2))

let g = (x) => (y) => base + x + y
console.log(g(3)(4))

let h = (a) => (b) => (c) => base + a + b + c
console.log(h(1)(2)(3))

let tag = "pre"
let s = (x) => (y) => tag + x + y
console.log(s("a")("b"))

function outer(x) {
  function inner(y) { return base + x + y }
  return inner
}
console.log(outer(5)(6))

let live = 1
let readLive = () => () => live
let probe = readLive()
live = 99
console.log(probe())

let deep = (x) => (y) => {
  let mid = base + x
  return (z) => {
    let inner = mid + y
    return inner + z
  }
}
console.log(deep(1)(2)(3))
