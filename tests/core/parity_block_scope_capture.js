// Node oracle for parity_block_scope_capture.tish (#247).
let out = []
function setup() {
  let v = 41
  out.push(() => v + 1)
}
setup()
console.log(out[0]())

// --- same, with an explicit return (used to work; must keep working) ---
let out2 = []
function setupReturning() {
  let v = 41
  out2.push(() => v + 1)
  return null
}
setupReturning()
console.log(out2[0]())

// --- escaping via a helper that stores the closure ---
let hooks = []
function addHook(h) { hooks.push(h) }
function register() {
  let label = "hook"
  addHook(() => label + "!")
}
register()
console.log(hooks[0]())

// --- escaping via index and member assignment ---
let slot = [null]
let holder = { f: null }
function assign() {
  let n = 7
  slot[0] = () => n * 2
  holder.f = () => n * 3
}
assign()
console.log(slot[0](), holder.f())

// --- an arrow's body block, not just a `fn` ---
let arrowOut = []
let build = () => {
  let tag = "arrow"
  arrowOut.push(() => tag)
}
build()
console.log(arrowOut[0]())

// --- the binding stays live: a write before the call returns is visible through the closure ---
let live = []
function mutating() {
  let c = 1
  live.push(() => c)
  c = 2
}
mutating()
console.log(live[0]())

// --- mutation THROUGH a captured reference (the beforeEach shape) ---
let mutators = []
function stateful() {
  let state = []
  mutators.push(() => { state.push(1); return state.length })
}
stateful()
console.log(mutators[0](), mutators[0](), mutators[0]())

// --- a body `let` shadowing an outer binding is not visible to the caller ---
let shadowed = "outer"
function shadowing() {
  let shadowed = "inner"
  return shadowed
}
console.log(shadowing(), shadowed)

// --- each call gets its own binding: two closures from two calls must not share ---
let perCall = []
function once(tag) {
  let captured = tag
  perCall.push(() => captured)
}
once("first")
once("second")
console.log(perCall[0](), perCall[1]())

// --- nested functions still resolve every ancestor's locals ---
function outerFn() {
  let a = 1
  function middle() {
    let b = 2
    return () => a + b
  }
  return middle()
}
console.log(outerFn()())
