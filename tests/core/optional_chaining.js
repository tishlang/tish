// MVP test: optional chaining ?. and nullish coalescing ?? (JS equivalent of optional_chaining.tish)
let x = null;
console.log(x ?? "default");
let y = 0;
console.log(y ?? 99);
let z = "hello";
console.log(z ?? "default");
const obj = { a: 1, b: 2 };
console.log(obj?.a);
console.log(obj?.c ?? "missing");
