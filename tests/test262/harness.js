// Test262 Harness for Tish
// Provides assertion functions compatible with test262 test format
// Note: Uses standalone functions since tish functions aren't objects

let _testCount = 0;
let _passCount = 0;
let _failCount = 0;

// Create assert as an object with methods
let assert = {
    sameValue: (actual, expected, message) => {
        _testCount += 1;
        if (actual !== expected) {
            _failCount += 1;
            console.error("FAIL:", message || "Expected " + expected + ", got " + actual);
        } else {
            _passCount += 1;
        }
    },
    notSameValue: (actual, unexpected, message) => {
        _testCount += 1;
        if (actual === unexpected) {
            _failCount += 1;
            console.error("FAIL:", message || "Expected value to differ from " + unexpected);
        } else {
            _passCount += 1;
        }
    },
    throws: (expectedErrorType, func, message) => {
        _testCount += 1;
        let threw = false;
        try {
            func();
        } catch (e) {
            threw = true;
        }
        if (!threw) {
            _failCount += 1;
            console.error("FAIL:", message || "Expected function to throw");
        } else {
            _passCount += 1;
        }
    }
};

function printTestResults() {
    console.log("=== Test Results ===");
    console.log("Total:", _testCount);
    console.log("Passed:", _passCount);
    console.log("Failed:", _failCount);
    if (_failCount === 0) {
        console.log("ALL TESTS PASSED");
    }
}
