// #716: body-level named function declarations. In VM general slot mode an uncaptured
// name is slot-bound (no frame-scope↔closure cycle); self/mutual recursion stays on the
// captured scope path. Behavior must be identical across interp / vm / native.

function basic() {
  function helper(a) { return a + 1 }
  return helper(41)
}
console.log(basic())

function capturesLocal() {
  let x = 10
  function addX(a) { return a + x }
  return addX(5)
}
console.log(capturesLocal())

function selfRec(n) {
  function fact(k) { if (k < 2) { return 1 } return k * fact(k - 1) }
  return fact(n)
}
console.log(selfRec(6))

function mutualRec(n) {
  function isEven(k) { if (k === 0) { return true } return isOdd(k - 1) }
  function isOdd(k) { if (k === 0) { return false } return isEven(k - 1) }
  return isEven(n)
}
console.log(mutualRec(10), mutualRec(7))

function blockScoped() {
  let got = 0
  {
    function inner() { return 42 }
    got = inner()
  }
  return got
}
console.log(blockScoped())

function asValue() {
  function double(v) { return v * 2 }
  let arr = [1, 2, 3]
  let out = arr.map(double)
  return out[0] + out[1] + out[2]
}
console.log(asValue())

function makeGetter() {
  let n = 7
  function get() { return n }
  return get
}
let g = makeGetter()
console.log(g())

function callsSiblingForward() {
  function a(v) { return b(v) + 1 }
  function b(v) { return v * 2 }
  return a(4)
}
console.log(callsSiblingForward())
