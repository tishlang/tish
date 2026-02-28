// Test262: Array literals

// Empty array
let empty = [];
assert.sameValue(empty.length, 0, "empty array length");

// Single element
let single = [42];
assert.sameValue(single.length, 1, "single element length");
assert.sameValue(single[0], 42, "single element access");

// Multiple elements
let nums = [1, 2, 3, 4, 5];
assert.sameValue(nums.length, 5, "multiple elements length");
assert.sameValue(nums[0], 1, "first element");
assert.sameValue(nums[2], 3, "middle element");
assert.sameValue(nums[4], 5, "last element");

// Mixed types
let mixed = [1, "two", true, null, [5, 6]];
assert.sameValue(mixed.length, 5, "mixed array length");
assert.sameValue(mixed[0], 1, "mixed - number");
assert.sameValue(mixed[1], "two", "mixed - string");
assert.sameValue(mixed[2], true, "mixed - boolean");
assert.sameValue(mixed[3], null, "mixed - null");
assert.sameValue(mixed[4].length, 2, "mixed - nested array");

// Nested arrays
let matrix = [[1, 2], [3, 4], [5, 6]];
assert.sameValue(matrix.length, 3, "matrix outer length");
assert.sameValue(matrix[0].length, 2, "matrix inner length");
assert.sameValue(matrix[0][0], 1, "matrix[0][0]");
assert.sameValue(matrix[1][1], 4, "matrix[1][1]");
assert.sameValue(matrix[2][0], 5, "matrix[2][0]");

// Array with expressions
let a = 5;
let b = 10;
let computed = [a, b, a + b, a * b];
assert.sameValue(computed[0], 5, "computed[0]");
assert.sameValue(computed[1], 10, "computed[1]");
assert.sameValue(computed[2], 15, "computed[2] (sum)");
assert.sameValue(computed[3], 50, "computed[3] (product)");

// Trailing comma
let trailing = [1, 2, 3,];
assert.sameValue(trailing.length, 3, "trailing comma length");

// Array with objects
let people = [
    { name: "Alice", age: 30 },
    { name: "Bob", age: 25 }
];
assert.sameValue(people.length, 2, "array of objects length");
assert.sameValue(people[0].name, "Alice", "first person name");
assert.sameValue(people[1].age, 25, "second person age");

// typeof array
assert.sameValue(typeof [], "object", "typeof array is object");
assert.sameValue(Array.isArray([]), true, "Array.isArray([])");
assert.sameValue(Array.isArray({}), false, "Array.isArray({})");

// Out of bounds
let arr = [1, 2, 3];
assert.sameValue(arr[10], null, "out of bounds is null");
assert.sameValue(arr[-1], null, "negative index is null");

printTestResults();
