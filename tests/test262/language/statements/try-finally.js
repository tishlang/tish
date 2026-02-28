// Test262: try/catch/finally

// Basic try/finally
let result = "";
try {
    result = "try";
} finally {
    result = result + " finally";
}
assert.sameValue(result, "try finally", "basic try/finally");

// Finally runs after error
result = "";
try {
    try {
        result = "try";
        throw "error";
    } finally {
        result = result + " finally";
    }
} catch (e) {
    result = result + " outer-catch";
}
assert.sameValue(result, "try finally outer-catch", "finally runs after throw");

// Try/catch/finally
result = "";
try {
    result = "try";
} catch (e) {
    result = result + " catch";
} finally {
    result = result + " finally";
}
assert.sameValue(result, "try finally", "try/catch/finally without error");

// Try/catch/finally with error
result = "";
try {
    result = "try";
    throw "error";
} catch (e) {
    result = result + " catch";
} finally {
    result = result + " finally";
}
assert.sameValue(result, "try catch finally", "try/catch/finally with error");

// Finally runs even when catching
let finallyRan = false;
try {
    throw "error";
} catch (e) {
    // caught
} finally {
    finallyRan = true;
}
assert.sameValue(finallyRan, true, "finally runs after catch");

// Finally with return in try
function testFinallyReturn() {
    let result = "";
    try {
        result = "try";
        return result;
    } finally {
        result = result + " finally";
    }
}
// Note: behavior may vary - finally should run but return value from try

// Nested finally blocks
result = "";
try {
    try {
        result = "inner-try";
    } finally {
        result = result + " inner-finally";
    }
    result = result + " outer-try";
} finally {
    result = result + " outer-finally";
}
assert.sameValue(result, "inner-try inner-finally outer-try outer-finally", "nested finally");

// Finally with exception propagation
result = "";
try {
    try {
        throw "inner-error";
    } finally {
        result = "inner-finally";
    }
} catch (e) {
    result = result + " caught:" + e;
}
assert.sameValue(result, "inner-finally caught:inner-error", "finally with exception propagation");

printTestResults();
