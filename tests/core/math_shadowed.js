// #203 SOUNDNESS: when `Math` is shadowed anywhere in the program, the whole-program
// `math_is_global` scan (name_rebinds_in_stmt) is false, so NO `Math.<fn>` lowers to the MathUnary
// intrinsic — every `Math.floor(...)` must call the user's shadowing value, on every backend.
// Byte-identical to node. Complements math_in_functions.tish (the unshadowed intrinsic path).

// Local shadow: `let Math = {...}` inside a function.
function localShadow() {
  let Math = { floor: function (x) { return x + 1000 } }
  return Math.floor(5)
}
console.log("local", localShadow())

// Parameter shadow: a param named `Math`.
function paramShadow(Math) { return Math.floor(5) }
console.log("param", paramShadow({ floor: function (x) { return x - 100 } }))

// The shadow disables the intrinsic PROGRAM-WIDE — this other function's real Math.floor must still
// be correct (just not intrinsic-lowered).
function realMath(n) {
  let s = 0
  for (let i = 0; i < n; i++) { s = s + Math.floor(Math.sqrt(i)) }
  return s
}
console.log("real", realMath(30))
