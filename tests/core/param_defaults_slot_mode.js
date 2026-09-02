// #727 twin: default parameter values (see param_defaults_slot_mode.tish).
// JS defaults fire on `undefined`; the tish fixture only ever omits args or passes
// null, and tish's missing-arg placeholder prints like null, so outputs align.

function f(a, b = 5) {
  let c = a + b
  return c
}
console.log(f(1))
console.log(f(1, 2))

function dep(a, b = a + 1) {
  let c = a + b
  return c
}
console.log(dep(3))
console.log(dep(3, 10))

function chain(a, b = a * 2, c = b + a) {
  let out = [a, b, c]
  return out[0] + "," + out[1] + "," + out[2]
}
console.log(chain(3))
console.log(chain(3, 10))

function g(a, b = 5) {
  function h() { return b }
  let c = a + h()
  return c
}
console.log(g(1))
console.log(g(1, 2))

function keepNull(a, b = 5) {
  let isNull = b === null
  return isNull
}
console.log(keepNull(1, null))
console.log(keepNull(1))
