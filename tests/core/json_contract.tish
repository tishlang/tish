// #180 JSON rewrite CONTRACT — byte-identical to node across interp/vm/native/cranelift/wasi/js.
// Pins the behavior the parse/stringify rewrite (serde parse + key interning, packed numeric
// arrays, shape-transition memo, ryu/itoa numbers) must preserve exactly. Each line is node-safe.

// --- Part 4: number formatting (ryu/itoa target) — shortest round-trip + JS exponent thresholds ---
console.log("n1", JSON.stringify([0, -0, 1, -1, 42, 1.5, -2.25, 0.1, 0.2, 0.3]))
console.log("n2", JSON.stringify([100000, 1000000, 1e20, 1e21, 1e-6, 1e-7, 1.5e300]))
console.log("n3", JSON.stringify([9007199254740991, 9007199254740993, 123456789.123456789]))
console.log("n4", JSON.stringify([3.0, 3.14159265358979, 2.718281828459045, 6.022e23]))
console.log("n5", JSON.stringify({ nan: 0 / 0, inf: 1 / 0, ninf: -1 / 0 }))

// --- Part 2: packed numeric arrays — all-number and mixed (downgrade) round-trip ---
let nums = []
for (let i = 0; i < 50; i++) { nums.push(i * 1.5) }
console.log("packed", JSON.parse(JSON.stringify(nums))[10], JSON.stringify(nums).length)
console.log("mixed", JSON.stringify([1, 2, "three", 4, true, null, 6]))

// --- Part 3: many identical-shape objects (transition memo target) — order + round-trip ---
let recs = []
for (let i = 0; i < 30; i++) { recs.push({ id: i, name: "r", value: i * 2, active: true, tags: [i] }) }
let rt = JSON.parse(JSON.stringify(recs))
console.log("shape", rt.length, rt[7].id, rt[7].value, Object.keys(rt[0]).join(","))

// --- escapes / unicode / control chars ---
console.log("esc", JSON.stringify("q\"bs\\slash/tab\tnl\ncr\rback\b"))
console.log("uni", JSON.parse("\"\\u0041\\u00e9\\u4e2d\""), JSON.stringify(""))

// --- nesting, string-aware scan, empties, duplicate keys, whitespace ---
console.log("nest", JSON.stringify(JSON.parse("{\"a\":{\"b\":{\"c\":[1,{\"d\":\"}{,\"}]}}}")))
console.log("empty", JSON.stringify({ o: {}, a: [], s: "" }))
console.log("dup", JSON.stringify(JSON.parse("{\"k\":1,\"k\":2,\"k\":3}")))
console.log("ws", JSON.stringify(JSON.parse("  { \"a\" : 1 , \"b\" : [ 2 , 3 ] }  ")))
console.log("bignest", JSON.parse("{\"a\":[1,2.5,-0,1e21],\"b\":\"he\\\"llo\\n\",\"c\":{\"d\":true,\"e\":null}}").a[3])
