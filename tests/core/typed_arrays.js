// Typed arrays — constructors, element coercion, statics, indexing, length, and iteration, identical
// across interp / VM / native / node. Coercion happens at construction (`new`, `.from`, `.of`); the
// backing is a plain array so indexing / `.length` / `for…of` / array methods all work. Outputs are
// elements/scalars (never the array object) so nothing depends on typed-array display formatting.

// Float32: rounds to f32 precision (0.1 is not representable in f32).
let f32 = new Float32Array([1.5, 0.1])
console.log(f32.length)
console.log(f32[0])              // 1.5 (exact)
console.log(f32[1] === 0.1)     // false — stored as the f32-rounded double
console.log(f32[1] < 0.1001 && f32[1] > 0.0999)  // true

// Float64: exact.
let f64 = new Float64Array([1.1, 2.2])
console.log(f64[0], f64[1])

// Uint8: truncate + wrap mod 256.
let u8 = new Uint8Array([300, -1, 256, 7, 3.9])
console.log(u8[0], u8[1], u8[2], u8[3], u8[4])   // 44 255 0 7 3

// Int8: signed wrap.
let i8 = new Int8Array([127, 128, -129, 256])
console.log(i8[0], i8[1], i8[2], i8[3])          // 127 -128 127 0

// Uint8Clamped: clamp to 0..255, round half to even.
let c = new Uint8ClampedArray([-5, 300, 2.5, 3.5, 0.5])
console.log(c[0], c[1], c[2], c[3], c[4])        // 0 255 2 4 0

// 16/32-bit wraps.
console.log(new Int16Array([32768, -32769])[0], new Int16Array([-32769])[0])  // -32768 32767
console.log(new Uint16Array([65536, 65537])[0], new Uint16Array([65537])[0])  // 0 1
console.log(new Int32Array([2147483648])[0])     // -2147483648
console.log(new Uint32Array([4294967296, 4294967297])[1])  // 1

// Length constructor → zero-filled.
let z = new Uint16Array(4)
console.log(z.length, z[0], z[3])                // 4 0 0

// Non-numeric → NaN → 0 for integer views.
let n = new Int32Array([null, "x"])
console.log(n[0], n[1])                          // 0 0

// Statics.
console.log(Uint8Array.of(1, 2, 300)[2])         // 44
console.log(Int32Array.from([1.9, 2.9, 3.9])[1]) // 2
console.log(Uint8Array.BYTES_PER_ELEMENT)        // 1
console.log(Float64Array.BYTES_PER_ELEMENT)      // 8
console.log(Int32Array.BYTES_PER_ELEMENT)        // 4

// Iteration + array methods (it's a real array underneath).
let sum = 0
for (let x of new Uint8Array([10, 20, 30])) { sum = sum + x }
console.log(sum)                                 // 60
console.log(new Float64Array([1, 2, 3, 4]).reduce((a, b) => a + b, 0))  // 10
