// Test262: for...of loops

// Iterate over array
let arr = [1, 2, 3, 4, 5];
let sum = 0;
for (let x of arr) {
    sum += x;
}
assert.sameValue(sum, 15, "for-of array sum");

// Iterate over string
let str = "hello";
let chars = "";
for (let c of str) {
    chars += c + "-";
}
assert.sameValue(chars, "h-e-l-l-o-", "for-of string iteration");

// Collect elements into array
let source = [10, 20, 30];
let collected = [];
for (let x of source) {
    collected.push(x);
}
assert.sameValue(collected.length, 3, "for-of collect - length");
assert.sameValue(collected[0], 10, "for-of collect[0]");
assert.sameValue(collected[2], 30, "for-of collect[2]");

// Transform elements
let nums = [1, 2, 3];
let doubled = [];
for (let n of nums) {
    doubled.push(n * 2);
}
assert.sameValue(doubled[0], 2, "for-of transform[0]");
assert.sameValue(doubled[1], 4, "for-of transform[1]");
assert.sameValue(doubled[2], 6, "for-of transform[2]");

// Nested for-of
let matrix = [[1, 2], [3, 4], [5, 6]];
sum = 0;
for (let row of matrix) {
    for (let cell of row) {
        sum += cell;
    }
}
assert.sameValue(sum, 21, "nested for-of matrix sum");

// Empty array
let count = 0;
for (let x of []) {
    count += 1;
}
assert.sameValue(count, 0, "for-of empty array");

// Single element
count = 0;
for (let x of [42]) {
    count += 1;
}
assert.sameValue(count, 1, "for-of single element");

// With const
let total = 0;
for (const val of [1, 2, 3]) {
    total += val;
}
assert.sameValue(total, 6, "for-of with const");

printTestResults();
