// #716 follow-up twin: read-only captured-param closure shapes
// (see captured_param_slot_mode.tish).

function handleEvent(payload) {
  function fmt(x) { return x + payload.tag }
  return fmt("v")
}
console.log(handleEvent({ tag: "t" }))

function withDefault(a, p = 5) {
  function get() { return p }
  let r = a + get()
  return r
}
console.log(withDefault(1))
console.log(withDefault(1, 2))

function escape(p) {
  function get() { return p }
  let unused = 0
  return get
}
let g = escape(4)
console.log(g())

function recur(n) {
  function dec() { return n - 1 }
  if (n === 0) { return 0 }
  let x = recur(dec())
  return x + n
}
console.log(recur(3))

function mixed(a, b) {
  function useA() { return a }
  let c = b * 2
  let r = useA() + c
  return r
}
console.log(mixed(1, 2))

function twoCaptured(a, b) {
  function sum() { return a + b }
  let r = sum() * 2
  return r
}
console.log(twoCaptured(3, 4))
