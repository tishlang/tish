// #440: Set / Map / String are iterable via spread, for-of, and call-spread across all backends.
// Before, spread/for-of over a Set/Map silently dropped elements (interp) or threw an uncatchable
// ConcatArray error (vm); spread over a string yielded [] / threw. interp/vm/native must all agree.
// Output is joined to plain strings so tish and node print identically (array inspect formats differ).
let s = new Set([1, 1, 2, 3])
let m = new Map([["a", 1], ["b", 2]])

console.log("set-spread", [...s].join(","))
console.log("map-spread", [...m].map(e => e.join(":")).join("|"))
console.log("str-spread", [..."abc"].join(","))
console.log("mixed", [0, ...s, 9].join(","))
console.log("callspread-max", Math.max(...s))

let acc = []
for (let x of s) { acc.push(x * 10) }
console.log("set-forof", acc.join(","))

let sacc = ""
for (let c of "xyz") { sacc = sacc + c + "." }
console.log("str-forof", sacc)

let keys = []
for (let e of m) { keys.push(e[0]) }
console.log("map-forof-keys", keys.join(","))
