// Regression: `continue` inside a `do { } while (cond)` jumps FORWARD to the condition test (the
// condition is emitted after the body). A backward jump resolved to a no-op and fell through into the
// body's already-unwound ExitBlock, crashing the VM. Valid in both tish and node.

// continue inside a nested block (unwinds body block + if-then block)
let d = 0
do {
  d = d + 1
  if (d === 2) { continue }
} while (d < 5)
console.log("basic", d)

// continue with no inner block (braceless if)
let a = 0
let sum = 0
do { a = a + 1; if (a === 2) continue; sum = sum + a } while (a < 5)
console.log("no-block", a, sum)

// continue and break together
let b = 0
let hits = 0
do { b = b + 1; if (b === 3) continue; if (b === 5) break; hits = hits + 1 } while (b < 10)
console.log("break", b, hits)

// nested do-while, continue in the inner loop
let outer = 0
let innerTotal = 0
do {
  outer = outer + 1
  let j = 0
  do { j = j + 1; if (j === 2) { continue } innerTotal = innerTotal + 1 } while (j < 3)
} while (outer < 2)
console.log("nested", outer, innerTotal)
