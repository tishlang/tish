// Test262: Array prototype accessor methods

// indexOf
let arr = [1, 2, 3, 2, 1];
assert.sameValue(arr.indexOf(2), 1, "indexOf first occurrence");
assert.sameValue(arr.indexOf(3), 2, "indexOf middle");
assert.sameValue(arr.indexOf(5), -1, "indexOf not found");
assert.sameValue(arr.indexOf(1), 0, "indexOf first element");

// indexOf with fromIndex
assert.sameValue(arr.indexOf(2, 2), 3, "indexOf with fromIndex");
assert.sameValue(arr.indexOf(1, 1), 4, "indexOf skip first");

// includes
arr = [1, 2, 3, 4, 5];
assert.sameValue(arr.includes(3), true, "includes found");
assert.sameValue(arr.includes(10), false, "includes not found");
assert.sameValue(arr.includes(1), true, "includes first");
assert.sameValue(arr.includes(5), true, "includes last");

// includes with fromIndex
assert.sameValue(arr.includes(1, 1), false, "includes with fromIndex - not found");
assert.sameValue(arr.includes(3, 2), true, "includes with fromIndex - found");

// slice
arr = [1, 2, 3, 4, 5];
let sliced = arr.slice(1, 4);
assert.sameValue(sliced.length, 3, "slice length");
assert.sameValue(sliced[0], 2, "slice[0]");
assert.sameValue(sliced[2], 4, "slice[2]");
assert.sameValue(arr.length, 5, "original unchanged");

// slice - negative indices
sliced = arr.slice(-3);
assert.sameValue(sliced.length, 3, "slice negative start");
assert.sameValue(sliced[0], 3, "slice negative[0]");

sliced = arr.slice(1, -1);
assert.sameValue(sliced.length, 3, "slice negative end");
assert.sameValue(sliced[2], 4, "slice negative end[2]");

// slice - no arguments (copy)
sliced = arr.slice();
assert.sameValue(sliced.length, 5, "slice copy length");
sliced[0] = 100;
assert.sameValue(arr[0], 1, "slice copy is independent");

// concat
let arr1 = [1, 2];
let arr2 = [3, 4];
let arr3 = [5, 6];
let concatted = arr1.concat(arr2);
assert.sameValue(concatted.length, 4, "concat two arrays");
assert.sameValue(concatted[2], 3, "concat[2]");

concatted = arr1.concat(arr2, arr3);
assert.sameValue(concatted.length, 6, "concat three arrays");
assert.sameValue(concatted[5], 6, "concat[5]");

// concat with values
concatted = arr1.concat(3, 4);
assert.sameValue(concatted.length, 4, "concat with values");
assert.sameValue(concatted[3], 4, "concat value");

// concat doesn't modify original
assert.sameValue(arr1.length, 2, "concat original unchanged");

// join
arr = [1, 2, 3];
assert.sameValue(arr.join(","), "1,2,3", "join with comma");
assert.sameValue(arr.join("-"), "1-2-3", "join with dash");
assert.sameValue(arr.join(""), "123", "join with empty");
assert.sameValue(arr.join(), "1,2,3", "join default");

// join with mixed types
arr = [1, "two", true, null];
assert.sameValue(arr.join("|"), "1|two|true|null", "join mixed types");

// empty array join
assert.sameValue([].join(","), "", "empty array join");

// single element join
assert.sameValue([42].join(","), "42", "single element join");

printTestResults();
