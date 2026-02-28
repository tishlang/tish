// Test262: Division operator (/)

// Basic division
assert.sameValue(12 / 4, 3, "12 / 4 should equal 3");
assert.sameValue(0 / 5, 0, "0 / anything is 0");
assert.sameValue(-12 / 4, -3, "negative / positive");
assert.sameValue(-12 / -4, 3, "negative / negative");
assert.sameValue(7 / 2, 3.5, "non-integer result");

// Float division
assert.sameValue(5.0 / 2.0, 2.5, "float / float");
assert.sameValue(1 / 3, 0.3333333333333333, "repeating decimal");

// Division by zero
assert.sameValue(1 / 0, Infinity, "positive / 0 is Infinity");
assert.sameValue(-1 / 0, -Infinity, "negative / 0 is -Infinity");

// Coercion
assert.sameValue(10 / true, 10, "number / true");
assert.sameValue("20" / 4, 5, "string number / number");
assert.sameValue(20 / "4", 5, "number / string number");

printTestResults();
