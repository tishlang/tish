// Array HOFs pass the source array as the trailing callback argument, matching JS:
//   map/filter/forEach/some/every/find/findIndex/findLast/findLastIndex/flatMap → (element, index, array)
//   reduce → (accumulator, element, index, array)
// Gated across interp/vm/native/cranelift/wasi/js + node. Valid in tish and node. The callbacks only
// READ the array arg (length / element-by-index) — no mutation-during-iteration, whose ordering is not
// worth pinning cross-backend. #247
let a = [5, 3, 8, 1, 9]

// map: element + array.length (3rd arg)
console.log("map", a.map((x, i, arr) => x + arr.length).join(","))
// map: index into the array arg
console.log("map-self", a.map((x, i, arr) => arr[i] * 10).join(","))
// filter: predicate uses array.length
console.log("filter", a.filter((x, i, arr) => x > arr.length).join(","))
// reduce: 4th arg is the array; fold length in
console.log("reduce", a.reduce((acc, x, i, arr) => acc + x + arr.length, 0))
// reduce no-init: seed is a[0], first call at index 1 with array arg
console.log("reduce-noinit", a.reduce((acc, x, i, arr) => acc + (arr.length - i)))
// forEach: accumulate array.length via a closure
let feTotal = 0
a.forEach((x, i, arr) => { feTotal = feTotal + arr.length })
console.log("forEach", feTotal)
// some / every over the array arg
console.log("some", a.some((x, i, arr) => i === arr.length - 1))
console.log("every", a.every((x, i, arr) => arr.length === 5))
// find / findIndex using array arg
console.log("find", a.find((x, i, arr) => x === arr[arr.length - 1]))
console.log("findIndex", a.findIndex((x, i, arr) => i === arr.length - 2))
// findLast / findLastIndex (iterate from the end; original index) using array arg
console.log("findLast", a.findLast((x, i, arr) => x < arr.length))
console.log("findLastIndex", a.findLastIndex((x, i, arr) => arr[i] > 4))
// flatMap: emit [element, array.length] pairs
console.log("flatMap", a.flatMap((x, i, arr) => [x, arr.length]).join(","))

// Regression: 1-arg and 2-arg callbacks (that ignore the extra arg) are unchanged.
console.log("one-arg", a.map(x => x * 2).join(","))
console.log("two-arg", a.map((x, i) => x + i).join(","))
console.log("reduce-2arg", a.reduce((s, x) => s + x, 0))
