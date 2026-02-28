// Test262: Boolean literals

// Boolean values
assert.sameValue(true, true, "true literal");
assert.sameValue(false, false, "false literal");

// Boolean equality
assert.sameValue(true === true, true, "true === true");
assert.sameValue(false === false, true, "false === false");
assert.sameValue(true === false, false, "true !== false");
assert.sameValue(false === true, false, "false !== true");

// Boolean in conditions
let result = "";
if (true) {
    result = "truthy";
}
assert.sameValue(result, "truthy", "true in condition");

result = "initial";
if (false) {
    result = "changed";
}
assert.sameValue(result, "initial", "false in condition");

// Boolean NOT
assert.sameValue(!true, false, "!true is false");
assert.sameValue(!false, true, "!false is true");
assert.sameValue(!!true, true, "!!true is true");
assert.sameValue(!!false, false, "!!false is false");

// Boolean AND
assert.sameValue(true && true, true, "true && true");
assert.sameValue(true && false, false, "true && false");
assert.sameValue(false && true, false, "false && true");
assert.sameValue(false && false, false, "false && false");

// Boolean OR
assert.sameValue(true || true, true, "true || true");
assert.sameValue(true || false, true, "true || false");
assert.sameValue(false || true, true, "false || true");
assert.sameValue(false || false, false, "false || false");

// Boolean to number
assert.sameValue(true + 0, 1, "true + 0 = 1");
assert.sameValue(false + 0, 0, "false + 0 = 0");
assert.sameValue(true + true, 2, "true + true = 2");

// typeof
assert.sameValue(typeof true, "boolean", "typeof true");
assert.sameValue(typeof false, "boolean", "typeof false");

// Boolean from expressions
assert.sameValue(1 === 1, true, "equal comparison returns true");
assert.sameValue(1 === 2, false, "unequal comparison returns false");
assert.sameValue(5 > 3, true, "greater than returns true");
assert.sameValue(5 < 3, false, "less than returns false");

printTestResults();
