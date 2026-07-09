// Node oracle for parity_value_fn_nonnumeric_arg.tish (#477).
function handler(req) {
  let status = 200
  let body = "ok"
  return status
}

function callWith(f, arg) {
  return f(arg)
}

console.log(callWith(handler, { method: "GET", path: "/" }))

function describe(req) {
  return req.method
}
console.log(callWith(describe, { method: "POST" }))

function square(n) {
  return n * n
}
console.log(square(7))
