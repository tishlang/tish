// #240 / #167: && and || follow JS semantics — they SHORT-CIRCUIT (the dead operand's side effects
// never run) and they RETURN THE OPERAND VALUE (not a coerced boolean). This pins the observable
// behavior across interp == vm == native == node. Valid in both tish and node.

function five() { return 5 }
function zero() { return 0 }
function emptyStr() { return "" }

// --- value-returning: a && b / a || b yield the operand, not a boolean ---
console.log("vr-and-tt", five() && 7)             // 5 truthy  -> 7
console.log("vr-and-ft", zero() && 7)             // 0 falsy   -> 0
console.log("vr-or-ft", zero() || 9)              // 0 falsy   -> 9
console.log("vr-or-tt", five() || 9)              // 5 truthy  -> 5
console.log("vr-and-str", emptyStr() && 7)        // "" falsy  -> ""
console.log("vr-chain", five() && zero() || 9)    // (5 && 0)=0 -> 0 || 9 -> 9
console.log("vr-nested", (five() && 7) || zero()) // (5 && 7)=7 truthy -> 7

// --- short-circuit: the right operand is NOT evaluated (no side effect) when the left decides it ---
let andCalls = 0
function andRight() { andCalls = andCalls + 1; return true }
let r1 = false && andRight()                      // false short-circuits -> andRight not called
let r2 = true && andRight()                       // true -> andRight called once
console.log("sc-and", r1, r2, andCalls)           // false true 1

let orCalls = 0
function orRight() { orCalls = orCalls + 1; return 7 }
let s1 = true || orRight()                         // true short-circuits -> orRight not called
let s2 = false || orRight()                        // false -> orRight called once
console.log("sc-or", s1, s2, orCalls)              // true 7 1

// --- short-circuit in a while condition: side-effect counts ---
let fCalls = 0
let gCalls = 0
let i = 0
function f() { fCalls = fCalls + 1; return i < 3 }
function g() { gCalls = gCalls + 1; return true }
while (f() && g()) { i = i + 1 }
console.log("while-and", i, fCalls, gCalls)        // 3 4 3 (g not called when f is false)

let qCalls = 0
function q() { qCalls = qCalls + 1; return false }
let n = 0
while ((n < 2) || q()) { n = n + 1 }
console.log("while-or", n, qCalls)                 // 2 1 (q only when n<2 is false)

// --- for + && and do-while + || conditions ---
let forCount = 0
for (let k = 0; k < 5 && forCount < 3; k = k + 1) { forCount = forCount + 1 }
console.log("for-and", forCount)                   // 3

let d = 0
let dq = 0
function dqf() { dq = dq + 1; return false }
do { d = d + 1 } while (d < 2 || dqf())
console.log("do-or", d, dq)                        // 2 1

// --- non-boolean truthiness in condition position ---
function andTruthy(v) {
  if (v && true) { return "truthy" }
  return "falsy"
}
console.log("and-0", andTruthy(0))
console.log("and-empty", andTruthy(""))
console.log("and-null", andTruthy(null))
console.log("and-1", andTruthy(1))
console.log("and-obj", andTruthy({}))

// --- nested (a && b) || c in condition position ---
function nested(a, b, c) {
  if ((a && b) || c) { return "yes" }
  return "no"
}
console.log("n-TTF", nested(true, true, false))
console.log("n-TFF", nested(true, false, false))
console.log("n-TFT", nested(true, false, true))
console.log("n-FxT", nested(false, true, true))
console.log("n-FxF", nested(false, true, false))
