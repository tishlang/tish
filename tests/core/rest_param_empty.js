// Regression: a rest parameter called with NO trailing args must bind to an
// empty array (`[]`), so `for-of` over it runs zero times. The bytecode VM used
// to do one spurious iteration (reading arr[0] -> null) and print NaN.
function sum(...a) {
  let t = 0
  for (let x of a) { t = t + x }
  return t
}
console.log(sum())
console.log(sum(1))
console.log(sum(1, 2, 3))

// rest parameter after a fixed parameter
function tail(first, ...rest) {
  let n = 0
  for (let r of rest) { n = n + r }
  return first + n
}
console.log(tail(10))
console.log(tail(10, 1, 2, 3))
