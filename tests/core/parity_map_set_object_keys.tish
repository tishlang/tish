// #466: Map/Set with object/array keys must preserve key IDENTITY (SameValueZero) on every
// backend. The interpreter bridged its values to fresh core allocations per call, so the stored
// key and the lookup probe never matched — get/has/delete failed for any reference-typed key.
// Gated across interp/vm/native + node. Misses print via truthiness (node yields `undefined`,
// tish yields `null` — both falsy) so output is backend-identical.

let m = new Map()
let k = { id: 1 }
let v = { tag: "val" }
m.set(k, v)

// Identity hit: same object key.
console.log("has-k", m.has(k))
console.log("get-k", m.get(k) ? m.get(k).tag : "miss")
// Round-trip identity: the stored value comes back as the SAME object.
console.log("get-identity", m.get(k) === v)
// A structurally-equal but DISTINCT object is a different key.
console.log("has-clone", m.has({ id: 1 }))
console.log("get-clone", m.get({ id: 1 }) ? "hit" : "miss")

// Identity is pointer-based, not content-based: mutating the key object doesn't lose the entry.
k.id = 42
console.log("has-mutated", m.has(k))
console.log("size-after-mutate", m.size)
// Re-setting the same key overwrites, never duplicates.
m.set(k, "second")
console.log("size-after-reset", m.size)
console.log("get-after-reset", m.get(k))

// keys() hands back the same object, not a copy.
let ks = []
for (const key of m.keys()) {
  ks.push(key)
}
console.log("keys-len", ks.length)
console.log("keys-identity", ks[0] === k)
console.log("keys-sees-mutation", ks[0].id)

// delete by object key.
console.log("delete-k", m.delete(k))
console.log("size-after-delete", m.size)

// Nested object as key (exercises child-object bridging).
let outer = { inner: { x: 1 } }
let m2 = new Map()
m2.set(outer.inner, "nested")
console.log("nested-get", m2.get(outer.inner))

// Set with an array member.
let s = new Set()
let a = [1, 2]
s.add(a)
console.log("set-has-a", s.has(a))
console.log("set-has-clone", s.has([1, 2]))
s.add(a)
console.log("set-size-dup", s.size)
console.log("set-delete", s.delete(a))
console.log("set-size-after", s.size)

// Map value identity for array values.
let m3 = new Map()
let arrv = [9, 8]
m3.set("key", arrv)
console.log("arr-value-identity", m3.get("key") === arrv)
