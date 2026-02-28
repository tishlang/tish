// Test262: Conditional (ternary) operator (? :)

// Basic conditional
assert.sameValue(true ? 1 : 2, 1, "true ? 1 : 2");
assert.sameValue(false ? 1 : 2, 2, "false ? 1 : 2");

// Truthy/falsy conditions
assert.sameValue(1 ? "yes" : "no", "yes", "1 is truthy");
assert.sameValue(0 ? "yes" : "no", "no", "0 is falsy");
assert.sameValue("" ? "yes" : "no", "no", "empty string is falsy");
assert.sameValue("hello" ? "yes" : "no", "yes", "non-empty string is truthy");
assert.sameValue(null ? "yes" : "no", "no", "null is falsy");

// Expression conditions
assert.sameValue((5 > 3) ? "greater" : "not greater", "greater", "comparison as condition");
assert.sameValue((5 < 3) ? "less" : "not less", "not less", "false comparison");

// Nested conditionals
let x = 5;
let result = x > 10 ? "large" : x > 5 ? "medium" : "small";
assert.sameValue(result, "small", "nested conditional");

x = 7;
result = x > 10 ? "large" : x > 5 ? "medium" : "small";
assert.sameValue(result, "medium", "nested conditional medium");

x = 15;
result = x > 10 ? "large" : x > 5 ? "medium" : "small";
assert.sameValue(result, "large", "nested conditional large");

// Conditional with different types
assert.sameValue(true ? 42 : "string", 42, "different types - number");
assert.sameValue(false ? 42 : "string", "string", "different types - string");

// Conditional with expressions in branches
assert.sameValue(true ? 1 + 2 : 3 + 4, 3, "expression in true branch");
assert.sameValue(false ? 1 + 2 : 3 + 4, 7, "expression in false branch");

// Conditional with function calls
function getTrue() { return true; }
function getFalse() { return false; }
assert.sameValue(getTrue() ? "yes" : "no", "yes", "function call condition - true");
assert.sameValue(getFalse() ? "yes" : "no", "no", "function call condition - false");

printTestResults();
