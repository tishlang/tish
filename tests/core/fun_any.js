// MVP test: fun and any (JS equivalent of fun_any.tish)
function double(x) { return x * 2; }
function add(a, b) { return a + b; }
const a = 10;
const b = 20;
console.log("double", a, "=", double(a));
console.log("add", a, b, "=", add(a, b));
function greet(name) {
  console.log("Hello,", name);
  return null;
}
greet("Tish");
