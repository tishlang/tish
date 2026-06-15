// Cross-backend parity: String.prototype.split(sep, limit). The `limit` argument must truncate the
// result to the first N pieces (JS semantics) — NOT keep the unsplit remainder in the last slot
// (Rust `splitn`) and NOT be ignored. Previously interp, vm, and the native backends each did this
// differently; now interp/vm/native/cranelift/wasi/js all agree with node. Valid in tish and node.

console.log("limit2", JSON.stringify("a,b,c,d".split(",", 2)))
console.log("nolimit", JSON.stringify("a,b,c,d".split(",")))
console.log("limit0", JSON.stringify("a,b,c,d".split(",", 0)))
console.log("over", JSON.stringify("a,b,c,d".split(",", 10)))
console.log("space2", JSON.stringify("one two three".split(" ", 2)))
console.log("single", JSON.stringify("x".split(",", 1)))
console.log("empty-parts", JSON.stringify("a,,b".split(",", 2)))
console.log("multichar", JSON.stringify("a::b::c".split("::", 2)))
console.log("limit1", JSON.stringify("a,b,c".split(",", 1)))
