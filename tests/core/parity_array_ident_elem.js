// Native codegen: a Value-typed identifier (object, or a global like NaN/Infinity) used BOTH as an
// array-literal element AND again in the same expression must be cloned, not moved (#247 native
// build failure: `[1, NaN].includes(NaN)`). Must stay bit-identical across all backends.
let o = { a: 1 }
let p = { a: 1 }
console.log("nan", [1, NaN].includes(NaN))
console.log("inf", [1, Infinity].includes(Infinity))
console.log("obj_same", [1, o].includes(o))
console.log("obj_diff", [1, o].includes(p))
console.log("str", ["a", "b"].includes("b"))
let arr = [10, 20, 30]
console.log("nested", [arr, arr].length)
console.log("reuse_idx", [o, o, o].indexOf(o))
