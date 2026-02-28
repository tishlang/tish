// Test262: Exponentiation operator (**)

// Basic exponentiation
assert.sameValue(2 ** 3, 8, "2 ** 3 should equal 8");
assert.sameValue(2 ** 0, 1, "anything ** 0 is 1");
assert.sameValue(2 ** 1, 2, "anything ** 1 is itself");
assert.sameValue(10 ** 2, 100, "10 ** 2 is 100");

// Negative bases
assert.sameValue((-2) ** 2, 4, "negative ** even is positive");
assert.sameValue((-2) ** 3, -8, "negative ** odd is negative");

// Fractional exponents
assert.sameValue(4 ** 0.5, 2, "4 ** 0.5 is sqrt(4)");
assert.sameValue(8 ** (1/3), 2, "8 ** (1/3) is cube root");
assert.sameValue(27 ** (1/3), 3, "27 ** (1/3) is 3");

// Negative exponents
assert.sameValue(2 ** -1, 0.5, "2 ** -1 is 0.5");
assert.sameValue(2 ** -2, 0.25, "2 ** -2 is 0.25");

// Large exponents
assert.sameValue(2 ** 10, 1024, "2 ** 10 is 1024");

// Coercion
assert.sameValue("2" ** 3, 8, "string ** number");
assert.sameValue(2 ** "3", 8, "number ** string");

printTestResults();
