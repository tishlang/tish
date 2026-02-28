// Test262: throw statement

// Basic throw and catch
let caught = false;
try {
    throw "error";
} catch (e) {
    caught = true;
}
assert.sameValue(caught, true, "basic throw caught");

// Throw string
let errorMsg = "";
try {
    throw "Something went wrong";
} catch (e) {
    errorMsg = e;
}
assert.sameValue(errorMsg, "Something went wrong", "throw string");

// Throw number
let errorNum = 0;
try {
    throw 42;
} catch (e) {
    errorNum = e;
}
assert.sameValue(errorNum, 42, "throw number");

// Throw object
let errorObj = null;
try {
    throw { code: 500, message: "Internal error" };
} catch (e) {
    errorObj = e;
}
assert.sameValue(errorObj.code, 500, "throw object - code");
assert.sameValue(errorObj.message, "Internal error", "throw object - message");

// Throw in function
function mightThrow(shouldThrow) {
    if (shouldThrow) {
        throw "error from function";
    }
    return "success";
}

let result = "";
try {
    result = mightThrow(false);
} catch (e) {
    result = "caught";
}
assert.sameValue(result, "success", "function without throw");

try {
    result = mightThrow(true);
} catch (e) {
    result = "caught: " + e;
}
assert.sameValue(result, "caught: error from function", "function with throw");

// Throw in nested function
function outer() {
    function inner() {
        throw "inner error";
    }
    inner();
    return "outer completed";
}

result = "";
try {
    result = outer();
} catch (e) {
    result = "caught: " + e;
}
assert.sameValue(result, "caught: inner error", "throw from nested function");

// Code after throw doesn't execute
let afterThrow = false;
try {
    throw "stop";
    afterThrow = true;
} catch (e) {
    // caught
}
assert.sameValue(afterThrow, false, "code after throw not executed");

printTestResults();
