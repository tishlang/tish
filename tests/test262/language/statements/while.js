// Test262: while loops

// Basic while loop
let count = 0;
while (count < 5) {
    count += 1;
}
assert.sameValue(count, 5, "basic while loop");

// While with false condition never executes
let executed = false;
while (false) {
    executed = true;
}
assert.sameValue(executed, false, "while false never executes");

// While with accumulator
let sum = 0;
let i = 1;
while (i <= 10) {
    sum += i;
    i += 1;
}
assert.sameValue(sum, 55, "sum 1 to 10");

// Nested while loops
let result = 0;
i = 0;
while (i < 3) {
    let j = 0;
    while (j < 3) {
        result += 1;
        j += 1;
    }
    i += 1;
}
assert.sameValue(result, 9, "nested while loops (3x3)");

// While with complex condition
let x = 10;
let y = 0;
while (x > 0 && y < 5) {
    x -= 1;
    y += 1;
}
assert.sameValue(x, 5, "while with AND condition - x");
assert.sameValue(y, 5, "while with AND condition - y");

// Single iteration
count = 0;
let iterations = 0;
while (count < 1) {
    iterations += 1;
    count += 1;
}
assert.sameValue(iterations, 1, "single iteration while");

// While building array
let arr = [];
i = 0;
while (i < 5) {
    arr.push(i);
    i += 1;
}
assert.sameValue(arr.length, 5, "while building array - length");
assert.sameValue(arr[0], 0, "while building array - first element");
assert.sameValue(arr[4], 4, "while building array - last element");

printTestResults();
