// Promise tests - JS equivalent for validation
// Run: node tests/modules/promise.js  (Node 18+ with top-level await, or use --input-type=module)
// Or: node -e "$(cat tests/modules/promise.js | sed 's/^/  /')" with async IIFE

(async () => {
// Promise(executor) - sync resolve
let p1 = new Promise((resolve, reject) => { resolve(42) })
let v1 = await p1
console.log("Promise sync resolve:", v1)

// Promise.resolve
let p2 = Promise.resolve(100)
let v2 = await p2
console.log("Promise.resolve:", v2)

// Promise.reject
let ok = false
try {
  await Promise.reject("err")
} catch (e) {
  ok = (e === "err")
}
console.log("Promise.reject caught:", ok)

// .then chain
let p3 = Promise.resolve(1).then(x => x + 1).then(x => x * 2)
let v3 = await p3
console.log(".then chain:", v3)

// .catch
let p4 = Promise.reject("fail").catch(e => "handled: " + e)
let v4 = await p4
console.log(".catch:", v4)

// Promise.all
let pa = Promise.all([Promise.resolve(1), Promise.resolve(2), Promise.resolve(3)])
let va = await pa
console.log("Promise.all:", va[0], va[1], va[2])

// Promise.race
let pr = Promise.race([Promise.resolve("fast"), Promise.resolve("slow")])
let vr = await pr
console.log("Promise.race:", vr)

console.log("Promise tests completed")
})()
