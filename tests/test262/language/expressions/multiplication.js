// Test262: Multiplication operator (*)

// Basic multiplication
assert.sameValue(3 * 4, 12, "3 * 4 should equal 12");
assert.sameValue(0 * 100, 0, "0 * anything is 0");
assert.sameValue(-3 * 4, -12, "negative * positive");
assert.sameValue(-3 * -4, 12, "negative * negative");
assert.sameValue(1 * 1, 1, "1 * 1 is 1");

// Float multiplication
assert.sameValue(2.5 * 4, 10, "float * int");
assert.sameValue(0.1 * 0.2, 0.020000000000000004, "float precision");

// Coercion
assert.sameValue(5 * true, 5, "number * true");
assert.sameValue(5 * false, 0, "number * false");
assert.sameValue(5 * null, 0, "number * null");
assert.sameValue("3" * 4, 12, "string number * number");
assert.sameValue(3 * "4", 12, "number * string number");

// Large numbers
assert.sameValue(1000000 * 1000000, 1000000000000, "large multiplication");

printTestResults();
