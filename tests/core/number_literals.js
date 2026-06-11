// Radix-prefixed integer literals: hex `0x`/`0X`, octal `0o`/`0O`, binary `0b`/`0B`,
// with optional `_` digit separators. JS semantics — non-negative integers; the value
// is backend-agnostic so interp / VM / native / node all agree.
console.log(0xff)
console.log(0xFF)
console.log(0X1a)
console.log(0o17)
console.log(0O7)
console.log(0b1010)
console.log(0B0)
console.log(0xdeadbeef)
console.log(0xFF_FF)
console.log(0b1111_0000)
console.log(255 & 0xff)
console.log(0xf0 | 0x0f)
console.log(0xff ^ 0x0f)
console.log(1 << 0x4)
console.log(0xffffffff >>> 0)
console.log(0x10 + 0o10 + 0b10)
let mask = 0xcafe
console.log(mask)
// Leading zero stays decimal (not legacy octal).
console.log(07)
console.log(0)
