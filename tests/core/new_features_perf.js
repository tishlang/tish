// Performance tests for new features

let iterations = 10000;
let start = 0;
let end = 0;

console.log("=== Performance tests for new features ===");
console.log("Iterations: " + iterations);

// array.sort() performance
start = Date.now();
for (let i = 0; i < iterations; i++) {
    let arr = [5, 2, 8, 1, 9, 3, 7, 4, 6];
    arr.sort((a, b) => a - b);
}
end = Date.now();
console.log("array.sort() with comparator: " + (end - start) + "ms");

// array.splice() performance
start = Date.now();
for (let i = 0; i < iterations; i++) {
    let arr = [1, 2, 3, 4, 5];
    arr.splice(2, 1, 'a', 'b');
}
end = Date.now();
console.log("array.splice(): " + (end - start) + "ms");

// Object.assign() performance
start = Date.now();
for (let i = 0; i < iterations; i++) {
    let target = { a: 1 };
    let source = { b: 2, c: 3 };
    Object.assign(target, source);
}
end = Date.now();
console.log("Object.assign(): " + (end - start) + "ms");

// Object.fromEntries() performance
start = Date.now();
for (let i = 0; i < iterations; i++) {
    let entries = [["a", 1], ["b", 2], ["c", 3]];
    Object.fromEntries(entries);
}
end = Date.now();
console.log("Object.fromEntries(): " + (end - start) + "ms");

// Array destructuring performance
start = Date.now();
for (let i = 0; i < iterations; i++) {
    let [a, b, c] = [1, 2, 3];
}
end = Date.now();
console.log("Array destructuring: " + (end - start) + "ms");

// Object destructuring performance
start = Date.now();
for (let i = 0; i < iterations; i++) {
    let { x, y, z } = { x: 1, y: 2, z: 3 };
}
end = Date.now();
console.log("Object destructuring: " + (end - start) + "ms");

// Array spread performance
start = Date.now();
for (let i = 0; i < iterations; i++) {
    let arr1 = [1, 2, 3];
    let arr2 = [...arr1, 4, 5];
}
end = Date.now();
console.log("Array spread: " + (end - start) + "ms");

// Object spread performance
start = Date.now();
for (let i = 0; i < iterations; i++) {
    let obj1 = { a: 1, b: 2 };
    let obj2 = { ...obj1, c: 3 };
}
end = Date.now();
console.log("Object spread: " + (end - start) + "ms");

// Function call spread performance
let sum3 = (a, b, c) => a + b + c;
start = Date.now();
for (let i = 0; i < iterations; i++) {
    let args = [1, 2, 3];
    sum3(...args);
}
end = Date.now();
console.log("Function call spread: " + (end - start) + "ms");

// Default parameters performance
let greet = (name = "World") => "Hello, " + name;
start = Date.now();
for (let i = 0; i < iterations; i++) {
    greet();
}
end = Date.now();
console.log("Default parameters (using default): " + (end - start) + "ms");

start = Date.now();
for (let i = 0; i < iterations; i++) {
    greet("Alice");
}
end = Date.now();
console.log("Default parameters (with arg): " + (end - start) + "ms");

console.log("=== Performance tests complete ===");
