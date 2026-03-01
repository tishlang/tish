// Type annotation tests - JS equivalent
// Note: JS doesn't have native type annotations, but TypeScript does

// Simple type annotations (JS uses runtime values)
let x = 42;
let name = "hello";
let flag = true;
let empty = null;

// Array types
let nums = [1, 2, 3];
let strs = ["a", "b", "c"];

// Function with typed parameters and return type
function add(a, b) {
    return a + b;
}

// Verify values work
console.log(add(x, 10));
console.log(name);
console.log(nums[1]);

// Const with type
const PI = 3.14159;
console.log(PI);
