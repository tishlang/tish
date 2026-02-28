// Test262: Subtraction operator (-)

// Basic subtraction
assert.sameValue(5 - 3, 2, "5 - 3 should equal 2");
assert.sameValue(0 - 0, 0, "0 - 0 should equal 0");
assert.sameValue(-5 - 3, -8, "negative - positive");
assert.sameValue(-5 - -3, -2, "negative - negative");
assert.sameValue(3 - 5, -2, "smaller - larger");

// Float subtraction
assert.sameValue(5.5 - 2.5, 3, "float subtraction");
assert.sameValue(0.3 - 0.1, 0.19999999999999998, "float precision");

// Coercion
assert.sameValue(5 - true, 4, "number - true");
assert.sameValue(5 - false, 5, "number - false");
assert.sameValue(5 - null, 5, "number - null");
assert.sameValue("10" - 5, 5, "string number - number");
assert.sameValue(10 - "5", 5, "number - string number");

printTestResults();
