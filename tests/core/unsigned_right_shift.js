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
// ToInt32 / ToUint32 are MODULO 2^32 (not a saturating cast) — the whole point of the fix.
console.log((4294967296 + 5) | 0)   // 2^32 wraps to 0, +5 -> 5
console.log(4294967296 & 1)         // ToInt32(2^32)=0 -> 0
console.log(4294967297 & 1)         // ToInt32(2^32+1)=1 -> 1
console.log(4294967296 >>> 0)       // ToUint32(2^32)=0
console.log(3.9 | 0)                // truncate toward zero -> 3
console.log(-3.9 | 0)               // -> -3
console.log(2147483648 | 0)         // 2^31 wraps to the negative side -> -2147483648
console.log(2147483648 >>> 0)       // ToUint32 keeps it positive -> 2147483648
console.log(-2147483648 >>> 0)      // -> 2147483648
console.log(4294967295 >> 0)        // ToInt32(2^32-1) = -1
console.log(4294967295 >> 1)        // sign-extends -> -1
console.log(1 << 32)                // shift count masks to 0 -> 1
console.log(1 << 33)                // -> 2
console.log(1 << 64)                // 64 & 31 = 0 -> 1
console.log(NaN | 0)                // ToInt32(NaN) = 0
console.log(Infinity | 0)           // 0
console.log((0 - Infinity) >>> 0)   // ToUint32(-Infinity) = 0
