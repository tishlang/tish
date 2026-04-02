// Test262: return statement

// Basic return
function getValue() {
    return 42;
}
assert.sameValue(getValue(), 42, "basic return value");

// Return with expression
function double(x) {
    return x * 2;
}
assert.sameValue(double(5), 10, "return with expression");

// Return string
function greet(name) {
    return "Hello, " + name;
}
assert.sameValue(greet("Universe"), "Hello, Universe", "return string");

// Early return
function earlyReturn(x) {
    if (x < 0) {
        return "negative";
    }
    return "non-negative";
}
assert.sameValue(earlyReturn(-5), "negative", "early return - negative");
assert.sameValue(earlyReturn(5), "non-negative", "early return - positive");

// Return without value
function noReturnValue() {
    let x = 1;
    return;
}
assert.sameValue(noReturnValue(), null, "return without value");

// Return from nested condition
function classify(x) {
    if (x < 0) {
        return "negative";
    } else if (x === 0) {
        return "zero";
    } else {
        return "positive";
    }
}
assert.sameValue(classify(-10), "negative", "classify -10");
assert.sameValue(classify(0), "zero", "classify 0");
assert.sameValue(classify(10), "positive", "classify 10");

// Return from loop
function findFirst(arr, target) {
    for (let i = 0; i < arr.length; i++) {
        if (arr[i] === target) {
            return i;
        }
    }
    return -1;
}
assert.sameValue(findFirst([1, 2, 3, 4, 5], 3), 2, "return from loop - found");
assert.sameValue(findFirst([1, 2, 3, 4, 5], 10), -1, "return from loop - not found");

// Return array
function makeArray() {
    return [1, 2, 3];
}
let arr = makeArray();
assert.sameValue(arr.length, 3, "return array - length");
assert.sameValue(arr[0], 1, "return array[0]");

// Return object
function makeObject() {
    return { x: 10, y: 20 };
}
let obj = makeObject();
assert.sameValue(obj.x, 10, "return object.x");
assert.sameValue(obj.y, 20, "return object.y");

printTestResults();
