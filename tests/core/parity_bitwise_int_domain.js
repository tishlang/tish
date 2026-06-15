// Int-domain bitwise lowering (#174): the native typed backend lowers a chain of bitwise/shift
// ops in the integer domain, erasing the intermediate `ToInt32`/`ToUint32`↔f64 round-trips. This
// must stay bit-identical to the interpreter, VM, cranelift, wasi and node. The key invariants:
//   - an f64 `*`/`+` inside a `>>>` is NOT a bitwise node, so it keeps its f64 arithmetic and only
//     ToUint32s the rounded product (the 2^53 rule: `h * 16777619` overflows 2^53 and must round
//     in f64 *before* ToUint32, exactly as V8 does);
//   - NaN / ±Infinity coerce to 0; negatives sign-extend under `>>` and wrap under `>>>`;
//   - shift counts mask mod 32.

// FNV-1a: the canonical chained-bitwise hot loop. `(h * prime) >>> 0` is the 2^53 case.
function fnv1a(n) {
  let h = 2166136261
  for (let i = 0; i < n; i = i + 1) {
    h = h ^ (i & 255)
    h = (h * 16777619) >>> 0
    h = ((h << 13) | (h >>> 19)) >>> 0
  }
  return h >>> 0
}

// Deeply nested chain (exercises the recursion staying in int domain across mixed ops). Operates
// on number-literal-seeded locals so it stays on the typed path the int-domain lowering targets.
function mix(n) {
  let a = 305419896
  let b = -1412567295
  let acc = 0
  for (let i = 0; i < n; i = i + 1) {
    a = (a + i) | 0
    acc = (acc ^ ((((a ^ b) << 3) | ((a & b) >>> 1)) ^ (((a | b) >> 2) & (b << 5)))) >>> 0
  }
  return acc >>> 0
}

// Untyped-param nested bitwise/unary/pow: these go through the BOXED/Value native path
// (`emit_bitwise_binop`/`emit_shift_binop`/unary/Pow), which previously generated
// `let Value::Number(a) = &(..)` blocks that shadowed across nesting and failed to compile
// (`error[E0308]`). Called with numeric args so every backend (incl. node) agrees.
function nested_boxed(a, b) { return ((a ^ b) << 3) | ((a & b) >>> 1) }
function nested_unary(a) { return ~(~a | 0) ^ -(-a) }
function nested_pow(a, b) { return (a ** b) | (a << 1) }
function deep_boxed(a, b) {
  return (((a ^ b) << 2) | ((a & b) >>> 1)) ^ (((a | b) >> 1) & ((a ^ 7) << 1))
}

console.log("fnv", fnv1a(100000))
console.log("mix", mix(100000))
console.log("nested_boxed", nested_boxed(305419896, -1412567295))
console.log("nested_unary", nested_unary(12345))
console.log("nested_pow", nested_pow(3, 5))
console.log("deep_boxed", deep_boxed(305419896, 271733878))
console.log("neg_xor", (-5) ^ 3)
console.log("mask_big", (0xFFFFFFFF & 0x0F))
console.log("ushr_signbit", (0x80000000 >>> 1))
console.log("ushr_neg1", ((-1) >>> 0))
console.log("shl_31", (1 << 31))
console.log("shl_wrap", (1 << 33))
console.log("shr_neg", ((-8) >> 2))
console.log("or_zero", (0xDEADBEEF | 0))
console.log("nan_or", ((0 / 0) | 0))
console.log("inf_or", ((1 / 0) | 0))
console.log("neginf_ushr", ((-1 / 0) >>> 0))
console.log("trunc_pos", (3.9 | 0))
console.log("trunc_neg", (-3.9 | 0))
console.log("rotate", (((255 << 24) | (128 << 16)) >>> 0))
