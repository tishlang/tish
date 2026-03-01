// Performance test for Date and Math functions

let iterations = 100000;

// Date.now() performance
let startDate = Date.now();
for (let i = 0; i < iterations; i = i + 1) {
    Date.now();
}
let dateTime = Date.now() - startDate;
console.log("Date.now() x " + iterations + ": " + dateTime + "ms");

// Math.random() performance
let startRandom = Date.now();
for (let j = 0; j < iterations; j = j + 1) {
    Math.random();
}
let randomTime = Date.now() - startRandom;
console.log("Math.random() x " + iterations + ": " + randomTime + "ms");

// Math.pow() performance
let startPow = Date.now();
for (let k = 0; k < iterations; k = k + 1) {
    Math.pow(2, 10);
}
let powTime = Date.now() - startPow;
console.log("Math.pow() x " + iterations + ": " + powTime + "ms");

// Trig functions performance
let startTrig = Date.now();
for (let l = 0; l < iterations; l = l + 1) {
    Math.sin(l);
    Math.cos(l);
}
let trigTime = Date.now() - startTrig;
console.log("sin+cos x " + iterations + ": " + trigTime + "ms");

// Math.log/exp performance
let startLogExp = Date.now();
for (let m = 0; m < iterations; m = m + 1) {
    Math.log(m + 1);
    Math.exp(m % 10);
}
let logExpTime = Date.now() - startLogExp;
console.log("log+exp x " + iterations + ": " + logExpTime + "ms");

console.log("Performance tests completed");
