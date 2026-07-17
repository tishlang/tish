// Large all-numeric array: build 5M, sum, in-place double, reverse, index-scan. If packed helps
// anywhere it's here (Vec<f64> vs Vec<Value>, unboxed fold/sort). Prints a checksum + timing.
let N = 5000000
let t0 = Date.now()
let a = []
let i = 0
while (i < N) { a.push(i * 3 - 1); i = i + 1 }
let s = 0
i = 0
while (i < N) { s = s + a[i]; i = i + 1 }
i = 0
while (i < N) { a[i] = a[i] * 2; i = i + 1 }
let s2 = a.reduce((x, y) => x + y, 0)
console.log("GAUNTLET packed_large " + (Date.now() - t0) + " " + ((s + s2) % 1000000007))
