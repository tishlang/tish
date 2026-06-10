// `Map`/`Set` `.values()` / `.keys()` / `.entries()` return real iterators: usable via
// `.next()` (→ { value, done }) and in `for…of` / spread, identical to node. Outputs are
// reduced to scalars so nothing depends on iterator-display formatting.
const m = new Map([["a", 1], ["b", 2], ["c", 3]])
let vs = 0
for (const v of m.values()) { vs = vs + v }
console.log(vs)
let ks = ""
for (const k of m.keys()) { ks = ks + k }
console.log(ks)
let es = ""
for (const e of m.entries()) { es = es + e[0] + e[1] }
console.log(es)
const it = m.values()
console.log(it.next().value, it.next().value)
console.log([...m.keys()].join(","))
console.log([...m.values()].length)
const s = new Set([10, 20, 30])
let ss = 0
for (const v of s.values()) { ss = ss + v }
console.log(ss)
console.log([...s.values()].join("-"))
console.log(new Map().values().next().done)
// Set.keys() / Set.entries() (entries yields [v, v] pairs).
console.log([...s.keys()].join("-"))
let se = ""
for (const e of s.entries()) { se = se + e[0] + ":" + e[1] + " " }
console.log(se.trim())
// Map.entries() pair via manual .next().
const me = m.entries()
const r = me.next().value
console.log(r[0], r[1])
// Iterators are STATEFUL: a partial .next() then for-of resumes from the current position.
const it2 = m.values()
it2.next()                                  // consume the first (1)
let rest = 0
for (const v of it2) { rest = rest + v }     // resumes -> 2 + 3
console.log(rest)
// Independent iterators from the same Map have separate positions.
const i1 = m.values()
const i2 = m.values()
i1.next()
console.log(i2.next().value)                 // i2 starts fresh -> 1
// next() past the end keeps returning done:true / value undefined.
const it3 = new Set([7]).values()
console.log(it3.next().value, it3.next().done, it3.next().done)
