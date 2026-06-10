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
