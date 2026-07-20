// #203: a ternary `cond ? a : b` inside a hot loop lowers to a branch-free `select` (like the
// straight-line path already does), instead of bailing the whole function to the interpreter.
// Byte-identical across all backends + node. Covers: simple ternary, comparison/logical conds,
// multiple ternaries per iteration, the result assigned/added/compared, and a nested ternary
// (control flow in an arm ⇒ conservatively bails ⇒ still correct via the VM).
function signfold(n) {
  let s = 0
  for (let i = 0; i < n; i++) {
    let v = (i % 3 === 0) ? i : -i
    s = s + v + (i < n / 2 ? 1 : 2)
  }
  return s
}
console.log("signfold", signfold(1000))

// abs-via-ternary + clamp-via-ternary in a loop
function absclamp(n) {
  let s = 0
  for (let i = -n; i < n; i++) {
    let a = i < 0 ? -i : i
    let c = a > 100 ? 100 : a
    s = s + c
  }
  return s
}
console.log("absclamp", absclamp(300))

// ternary result compared / used as a condition
function pick(n) {
  let t = 0
  for (let i = 0; i < n; i++) { if ((i % 2 === 0 ? 10 : 20) > 15) { t = t + 1 } }
  return t
}
console.log("pick", pick(500))

// nested ternary (arm is itself a ternary) — must still be correct (bails to VM)
function nested(n) {
  let s = 0
  for (let i = 0; i < n; i++) { s = s + (i % 3 === 0 ? 1 : (i % 3 === 1 ? 2 : 3)) }
  return s
}
console.log("nested", nested(600))
