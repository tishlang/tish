// Comma-separated declarators: `let a = 1, b = 2`, incl. uninitialized, for-init, and a
// destructuring declarator in the list. Each lowers to its own binding in the same scope.
let a = 1, b = 2, c = 3
console.log(a + b + c)
let x, y = 10
console.log(y)
let p = 1, q
q = 5
console.log(p + q)
let total = 0
for (let i = 0, n = 5; i < n; i++) { total = total + i }
console.log(total)
let [d, e] = [4, 5], f = 6
console.log(d + e + f)
function go() {
  let r = 7, s = 100
  return r + s
}
console.log(go())
const g = 1, h = 2
console.log(g + h)
