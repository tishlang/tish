// Regression: `for-of` over an EMPTY array must run the body zero times. The
// bytecode VM used to bottom-test the loop, so an empty array ran the body once
// (reading arr[0] -> null) and `continue` skipped the increment (infinite loop).

// empty array literal: body never runs
let total = 0
for (let x of []) { total = total + x }
console.log(total)

// empty array via a variable
let empty = []
let ran = false
for (let x of empty) { ran = true }
console.log(ran)

// `continue` reaches the increment (was: infinite loop)
let kept = []
for (let x of [1, 2, 3, 4]) {
  if (x === 2) { continue }
  kept.push(x)
}
console.log(kept.join(","))

// `break` still exits
let firsts = []
for (let x of [5, 6, 7]) {
  if (x === 7) { break }
  firsts.push(x)
}
console.log(firsts.join(","))

// empty inner loop inside a non-empty outer loop
let acc = 0
for (let i of [1, 2, 3]) {
  for (let j of []) { acc = acc + 100 }
  acc = acc + i
}
console.log(acc)
