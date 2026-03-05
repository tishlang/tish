// setTimeout tests - JS equivalent for validation
// Run: node tests/modules/settimeout.js

console.log("Before setTimeout")
setTimeout(() => { console.log("After 0ms (immediate)") }, 0)
console.log("After first setTimeout")

let id = setTimeout(() => { console.log("Delay completed") }, 1)
console.log("setTimeout returned id:", id)

clearTimeout(id)
console.log("setTimeout tests completed")
