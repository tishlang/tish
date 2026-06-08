// Hand-written JS reference for typed_strings.tish (type annotations stripped).
// Used by scripts/run_parity_compare.sh to check node parity. Output must match
// tests/core/typed_strings.tish.expected.

let a = "foo";
let b = "bar";

console.log(a + b);
console.log(a + b + a);
console.log(a + " " + b);

console.log(a + b === "foobar");
console.log(a === "foo");
console.log(a !== b);
console.log(a === b);

function join(x, y) {
  return x + y;
}
console.log(join("hello", "world"));
console.log(join(a, b) === "foobar");

let n = 3;
console.log("count=" + n);
console.log(a + n);
