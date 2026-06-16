// Mixed numeric relational (native typed path): when one operand lowers to a native f64 and the
// other is a boxed Value (e.g. an unannotated param only ever compared, like nsieve's `n`), the
// comparison is lowered to a native `f64 < value.as_number().unwrap_or(NaN)` instead of boxing the
// f64 side through `ops::lt`. Must stay bit-identical to interp/vm/cranelift/wasi/node. Numeric
// args throughout (the real hot-loop case), so node agrees too.
function countLt(n) { let c = 0; for (let i = 0; i < n; i++) { if (i < n) { c = c + 1 } } return c }
function bounds(lo, hi) { let s = 0; let x = lo; while (x <= hi) { s = s + x; x = x + 1 } return s }
function cmpAll(a) {
  let r = 0; let x = 3.5
  if (x < a) { r = r + 1 }
  if (x > a) { r = r + 2 }
  if (x <= a) { r = r + 4 }
  if (x >= a) { r = r + 8 }
  return r
}
// nsieve's exact shape: f64 counter vs boxed param in a strided inner loop.
function sieveCount(n) {
  let isP = []
  for (let i = 0; i < n; i++) { isP.push(true) }
  let count = 0
  for (let i = 2; i < n; i++) {
    if (isP[i]) { count = count + 1; let k = i + i; while (k < n) { isP[k] = false; k = k + i } }
  }
  return count
}
console.log("countLt", countLt(1000))
console.log("bounds", bounds(5, 100))
console.log("cmp_gt", cmpAll(2))
console.log("cmp_eqish", cmpAll(3.5))
console.log("cmp_lt", cmpAll(9))
console.log("sieve", sieveCount(10000))
