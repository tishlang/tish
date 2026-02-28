// Test262: Array.isArray

// Arrays
assert.sameValue(Array.isArray([]), true, "empty array");
assert.sameValue(Array.isArray([1, 2, 3]), true, "number array");
assert.sameValue(Array.isArray(["a", "b"]), true, "string array");
assert.sameValue(Array.isArray([[1], [2]]), true, "nested array");

// Non-arrays
assert.sameValue(Array.isArray({}), false, "empty object");
assert.sameValue(Array.isArray({ 0: "a", length: 1 }), false, "array-like object");
assert.sameValue(Array.isArray("hello"), false, "string");
assert.sameValue(Array.isArray(123), false, "number");
assert.sameValue(Array.isArray(true), false, "boolean");
assert.sameValue(Array.isArray(null), false, "null");
assert.sameValue(Array.isArray(function() {}), false, "function");

// Variable containing array
let arr = [1, 2, 3];
assert.sameValue(Array.isArray(arr), true, "variable array");

// Array from function
function getArray() {
    return [1, 2, 3];
}
assert.sameValue(Array.isArray(getArray()), true, "function return array");

// Array after modification
arr = [];
arr.push(1);
arr.push(2);
assert.sameValue(Array.isArray(arr), true, "modified array");

// Nested check
let nested = [[1, 2], [3, 4]];
assert.sameValue(Array.isArray(nested), true, "outer is array");
assert.sameValue(Array.isArray(nested[0]), true, "inner is array");
assert.sameValue(Array.isArray(nested[0][0]), false, "element is not array");

// With typeof comparison
let value = [1, 2, 3];
assert.sameValue(typeof value, "object", "typeof array is object");
assert.sameValue(Array.isArray(value), true, "but isArray is true");

// Conditional usage
function processValue(val) {
    if (Array.isArray(val)) {
        return "array with " + val.length + " elements";
    } else {
        return "not an array";
    }
}
assert.sameValue(processValue([1, 2, 3]), "array with 3 elements", "process array");
assert.sameValue(processValue("hello"), "not an array", "process string");
assert.sameValue(processValue({}), "not an array", "process object");

printTestResults();
