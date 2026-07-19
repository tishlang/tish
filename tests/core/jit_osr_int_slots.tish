// #514: top-level hot integer loops take the OSR loop-region JIT path (#408), which now keeps
// integer accumulators in i64 slots (Repr::I64Num) like the fn-body path (#511) instead of an
// f64 materialize per store. Covers: an FNV rolling hash at top level (the flagship win), an int
// slot ALSO read as f64 in the same loop, an out-of-2^32-range integer live-in (round-trips, int
// ops still ToInt32), and the entry live-in DEOPT guard (non-integer / NaN / -0 live-ins fall back
// to the interpreter). Identical output on interp / vm / native / cranelift / wasi / node.

// FNV-1a rolling hash, top level — the OSR int-slot fast path.
let h = 2166136261
let i = 0
while (i < 20000) {
  h = h ^ (i & 255)
  h = (h * 16777619) >>> 0
  h = ((h << 13) | (h >>> 19)) >>> 0
  i = i + 1
}
console.log("fnv", h)

// An int slot (a) read as f64 (added into a float sum) in the same loop.
let a = 305419896
let sum = 0
let j = 0
while (j < 1000) {
  a = (a << 1) ^ (a >>> 5)
  sum = sum + (a >>> 0)
  j = j + 1
}
console.log("mixed", a >>> 0, sum)

// Out-of-2^32-range integer live-in: round-trips through i64; int ops still ToInt32.
let big = 10000000000
let k = 0
while (k < 5) { big = (big ^ 1) + 0; k = k + 1 }
console.log("big", big >>> 0)

// Deopt guard: a non-integer live-in reaching an int-typed slot bails the region to the interpreter.
let f = 3.14
let m = 0
while (m < 4) { f = f ^ 7; m = m + 1 }
console.log("deopt-frac", f)

// Deopt guard: NaN live-in.
let g = 0 / 0
let p = 0
while (p < 4) { g = g | 3; p = p + 1 }
console.log("deopt-nan", g)

// Deopt guard: -0 live-in (must not be treated as +0).
let z = -0
let q = 0
while (q < 3) { z = z | 0; q = q + 1 }
console.log("deopt-negzero", 1 / z)
