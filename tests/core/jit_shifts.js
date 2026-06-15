// VM-JIT shift coverage (#168). Shifts live inside LOOP functions so the control-flow JIT
// (build_body_cfg) compiles them — the realistic hot path. Results must be bit-identical to the
// interpreter, native, and node. Covers JS count-masking (& 31, incl. i>31 in the loop), signed
// `>>` (sign-extending), and logical `>>>` past 2^31 (unsigned result). Valid in tish and node.

// FNV-1a-ish: a chain of `<<` + `>>>` accumulated in a hot loop.
function fnv(n) {
  let h = 2166136261
  for (let i = 0; i < n; i = i + 1) {
    h = h ^ i
    h = (h + ((h << 1) >>> 0) + ((h << 4) >>> 0) + ((h << 7) >>> 0) + ((h << 8) >>> 0) + ((h << 24) >>> 0)) >>> 0
  }
  return h >>> 0
}

// Signed arithmetic shift of a negative value (sign-extends), count masked to & 15 here.
function shrSum(n) {
  let s = 0
  for (let i = 0; i < n; i = i + 1) { s = s + ((-123456789) >> (i & 15)) }
  return s
}

// Logical shift; (-1) >>> k spans 2^32-1 down to 1 — exercises the unsigned->f64 convert.
function ushrSum(n) {
  let s = 0
  for (let i = 0; i < n; i = i + 1) { s = s + ((-1) >>> (i & 31)) }
  return s
}

// `1 << i` with i ranging past 31 — exercises JS count masking (i & 31) inside the JIT.
function shlMask(n) {
  let s = 0
  for (let i = 0; i < n; i = i + 1) { s = (s + (1 << i)) | 0 }
  return s
}

console.log("fnv", fnv(500))
console.log("shrSum", shrSum(64))
console.log("ushrSum", ushrSum(40))
console.log("shlMask", shlMask(40))
