// Test262: C-style for loops

// Basic for loop
let sum = 0;
for (let i = 0; i < 5; i++) {
    sum += i;
}
assert.sameValue(sum, 10, "basic for loop (0+1+2+3+4)");

// For loop with different step
sum = 0;
for (let i = 0; i < 10; i += 2) {
    sum += i;
}
assert.sameValue(sum, 20, "for loop step 2 (0+2+4+6+8)");

// For loop counting down
let result = "";
for (let i = 5; i > 0; i--) {
    result += i;
}
assert.sameValue(result, "54321", "for loop counting down");

// Nested for loops
let count = 0;
for (let i = 0; i < 3; i++) {
    for (let j = 0; j < 4; j++) {
        count += 1;
    }
}
assert.sameValue(count, 12, "nested for loops (3x4)");

// For loop with external variable
let x = 0;
for (x = 0; x < 5; x++) {
    // empty body
}
assert.sameValue(x, 5, "for loop with external variable");

// For loop building array
let arr = [];
for (let i = 0; i < 5; i++) {
    arr.push(i * 2);
}
assert.sameValue(arr.length, 5, "for loop array - length");
assert.sameValue(arr[0], 0, "for loop array[0]");
assert.sameValue(arr[2], 4, "for loop array[2]");
assert.sameValue(arr[4], 8, "for loop array[4]");

// For loop with multiple variables
let a = 0;
let b = 10;
for (let i = 0; i < 5; i++) {
    a += 1;
    b -= 1;
}
assert.sameValue(a, 5, "multiple variables - a");
assert.sameValue(b, 5, "multiple variables - b");

// Zero iterations
count = 0;
for (let i = 0; i < 0; i++) {
    count += 1;
}
assert.sameValue(count, 0, "zero iterations");

// Single iteration
count = 0;
for (let i = 0; i < 1; i++) {
    count += 1;
}
assert.sameValue(count, 1, "single iteration");

printTestResults();
