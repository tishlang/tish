// Performance test for builtin functions

let iterations = 100000;

// Array.isArray performance
let testArray = [1, 2, 3];
let testObj = { a: 1 };
let startIsArray = Date.now();
for (let i = 0; i < iterations; i = i + 1) {
    Array.isArray(testArray);
    Array.isArray(testObj);
}
let isArrayTime = Date.now() - startIsArray;
console.log("Array.isArray x " + (iterations * 2) + ": " + isArrayTime + "ms");

// String.fromCharCode performance
let startFromChar = Date.now();
for (let j = 0; j < iterations; j = j + 1) {
    String.fromCharCode(65 + (j % 26));
}
let fromCharTime = Date.now() - startFromChar;
console.log("String.fromCharCode x " + iterations + ": " + fromCharTime + "ms");

// process.cwd performance
let startCwd = Date.now();
for (let k = 0; k < iterations; k = k + 1) {
    process.cwd();
}
let cwdTime = Date.now() - startCwd;
console.log("process.cwd() x " + iterations + ": " + cwdTime + "ms");

// Combined: Math.sign/trunc
let startSignTrunc = Date.now();
for (let l = 0; l < iterations; l = l + 1) {
    Math.sign(l - 50000);
    Math.trunc(l / 100);
}
let signTruncTime = Date.now() - startSignTrunc;
console.log("sign+trunc x " + iterations + ": " + signTruncTime + "ms");

console.log("Builtin performance tests completed");
