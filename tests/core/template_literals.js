// Test template literal interpolation

// Basic interpolation
let name = "World";
console.log(`Hello, ${name}!`);

// Arithmetic in interpolation
let a = 5;
let b = 10;
console.log(`${a} + ${b} = ${a + b}`);

// Array method in interpolation
let nums = [1, 2, 3];
console.log(`Array: ${nums.join(", ")}`);

// Multiple interpolations
let first = "John";
let last = "Doe";
let age = 30;
console.log(`Name: ${first} ${last}, Age: ${age}`);

// Boolean expression
console.log(`Is adult: ${age >= 18}`);

// Empty string
let empty = "";
console.log(`Empty: [${empty}]`);

// Simple template (no interpolation)
console.log(`Plain template`);

// Object property in interpolation
let person = { name: "Alice", age: 25 };
console.log(`Person: ${person.name}, ${person.age}`);

// Function call in interpolation
function double(x) { return x * 2; }
console.log(`Double 5: ${double(5)}`);

// Ternary in interpolation
let x = 5;
console.log(`Value: ${x > 3 ? "big" : "small"}`);

// Multiline template
let multi = `Line 1
Line 2
Line 3`;
console.log(multi);

// Escaping
console.log(`Dollar: \$, Backtick: \`, Backslash: \\`);

// Nested object access
let data = { inner: { value: 42 } };
console.log(`Nested: ${data.inner.value}`);

// Chained method calls
let words = ["hello", "world"];
console.log(`Joined: ${words.join(" ").toUpperCase()}`);
