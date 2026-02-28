// Test262: Addition operator (+)

// Number addition
assert.sameValue(1 + 2, 3, "1 + 2 should equal 3");
assert.sameValue(0 + 0, 0, "0 + 0 should equal 0");
assert.sameValue(-5 + 3, -2, "negative + positive");
assert.sameValue(-5 + -3, -8, "negative + negative");
assert.sameValue(1.5 + 2.5, 4, "float addition");
assert.sameValue(0.1 + 0.2, 0.30000000000000004, "float precision");

// String concatenation
assert.sameValue("a" + "b", "ab", "string + string");
assert.sameValue("" + "", "", "empty strings");
assert.sameValue("hello" + " " + "world", "hello world", "multiple concatenation");

// String + number coercion
assert.sameValue("x" + 1, "x1", "string + number");
assert.sameValue(1 + "x", "1x", "number + string");
assert.sameValue("" + 42, "42", "empty string + number");

// String + boolean coercion
assert.sameValue("val:" + true, "val:true", "string + true");
assert.sameValue("val:" + false, "val:false", "string + false");

// Number + boolean coercion
assert.sameValue(1 + true, 2, "number + true");
assert.sameValue(1 + false, 1, "number + false");

// Null coercion
assert.sameValue(5 + null, 5, "number + null");
assert.sameValue("x" + null, "xnull", "string + null");

printTestResults();
