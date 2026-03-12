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

// Function returning void
function greet(msg) {
    console.log(msg);
}

// Verify values work
console.log(add(x, 10));
greet("Welcome!");
console.log(name);
console.log(nums[1]);

// Const with type
const PI = 3.14159;
console.log(PI);

// Object type annotation
let person = { name: "Alice", age: 30 };
console.log(person.name);

// Union types
let value = 42;
console.log(value);

// Function with rest params typed
function sum(...args) {
    let total = 0;
    for (const n of args) {
        total = total + n;
    }
    return total;
}
console.log(sum(1, 2, 3, 4, 5));
