// 20k medium numeric arrays with map/filter/reduce chains — allocation-heavy, the packed
// memory-footprint case. Checksum guards correctness across the flag.
let K = 20000
let M = 200
let t0 = Date.now()
let acc = 0
let k = 0
while (k < K) {
  let arr = []
  let j = 0
  while (j < M) { arr.push((j * k + 7) % 97); j = j + 1 }
  let r = arr.map((x) => x * 2).filter((x) => x > 50).reduce((a, b) => a + b, 0)
  acc = (acc + r) % 1000000007
  k = k + 1
}
console.log("GAUNTLET packed_many " + (Date.now() - t0) + " " + acc)
