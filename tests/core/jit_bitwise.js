// #168: bitwise/shift chains inside hot-loop functions — the shapes the VM JIT now keeps in
// integer registers (typed value repr I32/U32) instead of paying int→f64→int round-trips per
// op. Loop bodies force build_body_cfg compilation; values cross StoreLocal/compare/Return
// boundaries so every repr materialization path runs. Covers on the JIT path: shift counts
// past 31 (masking), negative counts, negative LHS, NaN / Infinity / fractional LHS,
// `-1 >>> 0`, `~` chains, mixed int/float expressions, and the f64-rounding multiply
// `(h * 16777619) >>> 0` (the product rounds past 2^53 — deliberately f64, matching V8; the
// checksum pins that agreement). Identical output across interp/vm/native + node.

// FNV-1a-style rolling hash: the canonical (h ^ b) * K >>> 0 with a rotate mix.
function fnvish(n) {
  let h = 2166136261
  let i = 0
  while (i < n) {
    h = h ^ (i & 255)
    h = (h * 16777619) >>> 0
    h = ((h << 13) | (h >>> 19)) >>> 0
    i = i + 1
  }
  return h
}
console.log(fnvish(1000))

// Shift-count masking (counts >= 32 wrap mod 32), negative counts (ToUint32 low 5 bits),
// negative LHS sign behavior for << >> >>>.
function shifts(x) {
  let acc = 0
  acc = acc + (x << 33)      // count 33 & 31 = 1
  acc = acc + (x >> 33)
  acc = acc + (x >>> 33)
  acc = acc + (x << -1)      // -1 & 31 = 31
  acc = acc + (x >> -1)
  acc = acc + ((0 - x) >> 2)
  acc = acc + ((0 - x) >>> 2)
  return acc
}
console.log(shifts(7))
console.log(shifts(1))

// Non-finite / fractional LHS: ToInt32(NaN) = 0, ToInt32(Inf) = 0, fractions truncate.
function oddInputs(a, b) {
  let nan = a / b            // 0/0 -> NaN
  let inf = 1 / b            // 1/0 -> Infinity
  let r = 0
  let i = 0
  while (i < 3) {
    r = r + ((nan | 0) + (inf | 0) + (2.9 << 1) + (-2.9 >> 1))
    i = i + 1
  }
  return r
}
console.log(oddInputs(0, 0))

// -1 >>> 0 = 4294967295 (U32 repr materializes UNSIGNED); ~ chains stay in i32.
function unsignedAndNot(n) {
  let x = 0 - n
  let big = x >>> 0
  let t = ~(~(~n))
  return big + t
}
console.log(unsignedAndNot(1))

// Mixed int/float boundary: an int-chain value feeding float arithmetic and comparisons.
function mixed(n) {
  let s = 0
  let i = 0
  while (i < n) {
    let m = (i ^ 21) & 1023
    s = s + m * 1.5
    if ((m | 0) > 500) { s = s - 1 }
    i = i + 1
  }
  return s
}
console.log(mixed(2000))

// Ternary over int-chain arms (build_body select path).
function pick(a, b) {
  return a & 1 ? (b << 2) : (b >>> 1)
}
console.log(pick(3, 10), pick(2, 10))
