// Test262: Modulus operator (%)

// Basic modulus
assert.sameValue(10 % 3, 1, "10 % 3 should equal 1");
assert.sameValue(9 % 3, 0, "9 % 3 should equal 0");
assert.sameValue(5 % 10, 5, "smaller % larger");
assert.sameValue(0 % 5, 0, "0 % anything is 0");

// Negative modulus
assert.sameValue(-10 % 3, -1, "negative % positive");
assert.sameValue(10 % -3, 1, "positive % negative");
assert.sameValue(-10 % -3, -1, "negative % negative");

// Float modulus
assert.sameValue(5.5 % 2, 1.5, "float modulus");
assert.sameValue(10 % 2.5, 0, "int % float");

// Coercion
assert.sameValue(10 % true, 0, "number % true");
assert.sameValue("10" % 3, 1, "string number % number");
assert.sameValue(10 % "3", 1, "number % string number");

printTestResults();
