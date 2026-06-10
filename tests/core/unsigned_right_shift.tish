// `>>>` unsigned right shift + the bitwise family with JS ToInt32/ToUint32 (modulo 2^32,
// not a saturating cast) — exact across interp / VM / native / node, incl. large hash values.
console.log(-1 >>> 0)
console.log(-1 >>> 1)
console.log(256 >>> 4)
console.log(5 >>> 0)
console.log(4294967295 >>> 0)
console.log(1 << 4)
console.log(-8 >> 1)
console.log(1 << 35)
console.log(255 & 4294967295)
console.log(2166136261 ^ 16777619)
console.log((2166136261 * 16777619) >>> 0)
console.log(~0)
console.log(~5)
console.log(5 >>> -1)
// FNV-1a-style hashing loop: every op stays in the uint32 range via `>>> 0`.
function hash(n) {
  let h = 2166136261
  for (let i = 0; i < n; i++) {
    h = h ^ (i & 255)
    h = (h * 16777619) >>> 0
    h = (h << 13) | (h >>> 19)
  }
  return h >>> 0
}
console.log(hash(1000))
