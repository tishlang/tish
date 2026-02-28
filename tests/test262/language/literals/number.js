// Test262: Number literals

// Integer literals
assert.sameValue(0, 0, "zero");
assert.sameValue(1, 1, "one");
assert.sameValue(42, 42, "positive integer");
assert.sameValue(1000000, 1000000, "large integer");

// Negative numbers
assert.sameValue(-1, -1, "negative one");
assert.sameValue(-42, -42, "negative integer");
assert.sameValue(-1000000, -1000000, "large negative");

// Floating point
assert.sameValue(3.14, 3.14, "pi approximation");
assert.sameValue(0.5, 0.5, "half");
assert.sameValue(0.0, 0, "zero point zero");
assert.sameValue(1.0, 1, "one point zero equals one");

// Small decimals
assert.sameValue(0.001, 0.001, "small decimal");
assert.sameValue(0.123456789, 0.123456789, "many decimal places");

// Negative floats
assert.sameValue(-3.14, -3.14, "negative pi");
assert.sameValue(-0.5, -0.5, "negative half");

// Special values
assert.sameValue(typeof Infinity, "number", "Infinity is number");
assert.sameValue(typeof NaN, "number", "NaN is number");
assert.sameValue(Infinity > 1000000, true, "Infinity is large");
assert.sameValue(-Infinity < -1000000, true, "negative Infinity is small");

// NaN comparisons
assert.sameValue(NaN === NaN, false, "NaN !== NaN");
assert.sameValue(isNaN(NaN), true, "isNaN(NaN)");

// Number type
assert.sameValue(typeof 42, "number", "typeof integer");
assert.sameValue(typeof 3.14, "number", "typeof float");
assert.sameValue(typeof -5, "number", "typeof negative");

// Arithmetic with literals
assert.sameValue(2 + 3, 5, "2 + 3");
assert.sameValue(10 - 4, 6, "10 - 4");
assert.sameValue(3 * 4, 12, "3 * 4");
assert.sameValue(15 / 3, 5, "15 / 3");

printTestResults();
