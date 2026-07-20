// #539: all eight ECMAScript Math value constants must be present and byte-identical to node on
// every backend (interp/vm/native/cranelift/wasi/js). LN2/LN10/LOG2E/LOG10E/SQRT2/SQRT1_2 were
// previously missing (returned null).
console.log(Math.PI, Math.E, Math.LN2, Math.LN10)
console.log(Math.LOG2E, Math.LOG10E, Math.SQRT2, Math.SQRT1_2)
// used in expressions (the values, not just presence)
console.log(Math.SQRT2 * Math.SQRT1_2, Math.LOG2E * Math.LN2, Math.LN10 / Math.LN2)
