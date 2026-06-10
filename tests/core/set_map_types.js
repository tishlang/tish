// `new Set(...)` and `new Map(...)` — constructors, methods, and the computed `.size`, identical
// across interp / VM / native / node. Iteration uses `.values()` / `.keys()` (which return arrays);
// outputs are reduced to scalars so nothing depends on collection-display formatting.

// ── Set ──────────────────────────────────────────────────────────────────────
let s = new Set([1, 2, 2, 3, 3, 3])
console.log(s.size)        // 3 (deduped)
console.log(s.has(2))      // true
console.log(s.has(9))      // false
s.add(4)
s.add(2)                   // duplicate — no-op
console.log(s.size)        // 4
console.log(s.delete(1))   // true
console.log(s.delete(1))   // false (already removed)
console.log(s.size)        // 3

let st = 0
for (let v of s.values()) { st = st + v }
console.log(st)            // 2 + 3 + 4 = 9

s.clear()
console.log(s.size)        // 0

// NaN collapses to a single member (SameValueZero).
let sn = new Set([NaN, NaN])
console.log(sn.size)       // 1
console.log(sn.has(NaN))   // true

// ── Map ──────────────────────────────────────────────────────────────────────
let m = new Map([["a", 1], ["b", 2]])
console.log(m.size)        // 2
console.log(m.get("a"))    // 1
console.log(m.has("b"))    // true
console.log(m.has("z"))    // false
m.set("c", 3)
m.set("a", 9)              // update existing key
console.log(m.size)        // 3
console.log(m.get("a"))    // 9
console.log(m.delete("b")) // true
console.log(m.size)        // 2

let mt = 0
for (let v of m.values()) { mt = mt + v }
console.log(mt)            // 9 + 3 = 12

let mk = ""
for (let k of m.keys()) { mk = mk + k }
console.log(mk)            // "ac" (insertion order; b removed)

m.clear()
console.log(m.size)        // 0

// Numeric keys.
let mn = new Map()
mn.set(1, "one")
mn.set(2, "two")
console.log(mn.get(1))     // "one"
console.log(mn.size)       // 2
