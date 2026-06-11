// String relational operators (`<` `<=` `>` `>=`) compare lexicographically
// when both operands are strings (JS semantics). Regression test for #99.

// ── literal forms (constant-folded) ────────────────────────────────────
console.log("a" < "b", "b" < "a", "a" <= "a", "b" > "a");
console.log("apple" < "banana", "apple" < "apply", "app" < "apple");
console.log("Z" < "a", "10" < "9", "" < "a");
console.log("a" >= "a", "b" >= "a", "a" > "a", "a" <= "b");

// ── variable forms (runtime path) ──────────────────────────────────────
let x = "a";
let y = "b";
console.log(x < y, y < x, x <= x, y > x);

let lo = "a";
let hi = "z";
let c = "m";
console.log(c >= lo, c <= hi, c >= lo ? c <= hi : false);

// ── numeric comparison is unaffected ───────────────────────────────────
console.log(1 < 2, 2 < 1, 3 <= 3, 5 > 4);
