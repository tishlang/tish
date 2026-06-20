// #176: native numeric globals — boxed and native callers share one thread_local PRNG state.
let counter = 10

function bump() {
  counter = (counter * 3 + 7) % 1000
  return counter
}

function read() {
  return counter
}

function run() {
  let a = bump()
  let b = read()
  let c = bump()
  return a * 10000 + b * 100 + c
}

console.log("native_numeric_global " + run())
