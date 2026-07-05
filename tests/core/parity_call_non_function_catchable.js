// Calling a non-callable value throws a CATCHABLE TypeError on every backend + node — it must never
// abort the process. The native (rust AOT) backend used to `panic!` in `value_call` (an uncatchable
// abort) for a method call on null whose read returned null; #381 makes it park a catchable TypeError
// like the VM/interpreter/cranelift/wasi/node. Only `e.name` is asserted (messages differ across
// engines). Valid in tish and node.

// method on null (the read yields null, then the call target is non-callable)
let r1 = "no-throw"
try {
  let z = null
  z.foo()
} catch (e) {
  r1 = e.name
}
console.log("null-method", r1)

// calling a plain number
let r2 = "no-throw"
try {
  let n = 5
  n()
} catch (e) {
  r2 = e.name
}
console.log("number-call", r2)

// calling a string
let r3 = "no-throw"
try {
  let s = "hi"
  s()
} catch (e) {
  r3 = e.name
}
console.log("string-call", r3)

// the program keeps running after each catch (no abort)
console.log("survived", true)

// normal calls are unaffected
function add(a, b) { return a + b }
console.log("normal-call", add(2, 3))
let o = { greet: () => "hi" }
console.log("method-call", o.greet())
