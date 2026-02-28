// Test262: try/catch exception handling

// Basic try/catch
let result = "";
try {
    result = "try block";
} catch (e) {
    result = "catch block";
}
assert.sameValue(result, "try block", "try without error");

// Catch thrown error
result = "";
try {
    throw "error";
    result = "after throw";
} catch (e) {
    result = "caught: " + e;
}
assert.sameValue(result, "caught: error", "catch thrown error");

// Error object in catch
let caughtError = null;
try {
    throw "test error";
} catch (e) {
    caughtError = e;
}
assert.sameValue(caughtError, "test error", "error object in catch");

// Nested try/catch
result = "";
try {
    try {
        throw "inner error";
    } catch (e) {
        result = "inner caught: " + e;
        throw "outer error";
    }
} catch (e) {
    result = result + ", outer caught: " + e;
}
assert.sameValue(result, "inner caught: inner error, outer caught: outer error", "nested try/catch");

// Try/catch in function
function safeDivide(a, b) {
    try {
        if (b === 0) {
            throw "division by zero";
        }
        return a / b;
    } catch (e) {
        return "error: " + e;
    }
}
assert.sameValue(safeDivide(10, 2), 5, "safeDivide success");
assert.sameValue(safeDivide(10, 0), "error: division by zero", "safeDivide error");

// Multiple statements in try
let step = 0;
try {
    step = 1;
    step = 2;
    step = 3;
} catch (e) {
    step = -1;
}
assert.sameValue(step, 3, "multiple statements in try");

// Error thrown midway
step = 0;
try {
    step = 1;
    throw "stop";
    step = 2;
} catch (e) {
    step = step + 10;
}
assert.sameValue(step, 11, "error thrown midway (1 + 10)");

// Try/catch with loop
let count = 0;
for (let i = 0; i < 5; i++) {
    try {
        if (i === 2) {
            throw "skip " + i;
        }
        count += 1;
    } catch (e) {
        // skip this iteration
    }
}
assert.sameValue(count, 4, "try/catch in loop (skipped i=2)");

printTestResults();
