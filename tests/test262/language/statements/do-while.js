// Test262: do...while loops

// Basic do-while
let count = 0;
do {
    count += 1;
} while (count < 5);
assert.sameValue(count, 5, "basic do-while loop");

// Do-while executes at least once
let executed = false;
do {
    executed = true;
} while (false);
assert.sameValue(executed, true, "do-while executes at least once");

// Compare with while (which wouldn't execute)
executed = false;
while (false) {
    executed = true;
}
assert.sameValue(executed, false, "while false never executes (comparison)");

// Do-while with accumulator
let sum = 0;
let i = 1;
do {
    sum += i;
    i += 1;
} while (i <= 10);
assert.sameValue(sum, 55, "do-while sum 1 to 10");

// Nested do-while
let result = 0;
i = 0;
do {
    let j = 0;
    do {
        result += 1;
        j += 1;
    } while (j < 3);
    i += 1;
} while (i < 3);
assert.sameValue(result, 9, "nested do-while (3x3)");

// Do-while with complex condition
let x = 10;
let y = 0;
do {
    x -= 1;
    y += 1;
} while (x > 0 && y < 5);
assert.sameValue(x, 5, "do-while with AND condition - x");
assert.sameValue(y, 5, "do-while with AND condition - y");

// Single iteration with initially false condition
count = 10;
let iterations = 0;
do {
    iterations += 1;
} while (count < 5);
assert.sameValue(iterations, 1, "do-while single iteration even with false condition");

printTestResults();
