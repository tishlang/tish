// #203: two-arg Math.<fn> (max/min/pow/atan2/hypot) called inside a function must compute the correct
// result and lower to the MathBinary intrinsic when Math is the unshadowed global, so numeric kernels
// that clamp/exponentiate JIT instead of bailing. Byte-identical across all backends + node. Covers
// the NaN / ±0 / ±Infinity edge semantics, 3-arg fallback (variadic max/min), and pow edge cases.

function clampSum(n) {
  let s = 0
  for (let i = 0; i < n; i++) {
    s = s + Math.max(i % 10, 3) - Math.min(i % 10, 7) + Math.pow(i % 4, 2)
  }
  return s
}
console.log("clamp", clampSum(100))

function geo(n) {
  let d = 0
  for (let i = 0; i < n; i++) { d = d + Math.hypot(i, i + 1) + Math.atan2(i, 2) }
  return Math.floor(d * 100)
}
console.log("geo", geo(50))

// Edge semantics (must match JS exactly): NaN propagation, +0 vs -0 preference, ±Infinity.
function edges() {
  let out = []
  out.push(Math.max(0 / 0, 5))          // NaN
  out.push(Math.min(5, 0 / 0))          // NaN
  out.push(1 / Math.max(-0, 0))         // +Infinity (max(-0,+0) = +0)
  out.push(1 / Math.min(-0, 0))         // -Infinity (min(-0,+0) = -0)
  out.push(Math.max(1 / 0, 100))        // Infinity
  out.push(Math.min(-1 / 0, -100))      // -Infinity
  out.push(Math.pow(2, 10))             // 1024
  out.push(Math.pow(4, 0.5))            // 2
  out.push(Math.pow(0 / 0, 0))          // 1 (pow(NaN,0)===1)
  return out.join(",")
}
console.log("edges", edges())

// 3-arg max/min (variadic) must still work (falls back off the 2-arg intrinsic).
function variadic() { return Math.max(1, 9, 4) + Math.min(8, 2, 5) }
console.log("variadic", variadic())

// Top-level (already worked).
console.log("toplevel", Math.max(3, 7), Math.min(3, 7), Math.pow(3, 3))
