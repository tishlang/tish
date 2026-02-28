// Test262: Comparison operators (<, >, <=, >=)

// Less than
assert.sameValue(1 < 2, true, "1 < 2");
assert.sameValue(2 < 1, false, "2 < 1");
assert.sameValue(1 < 1, false, "1 < 1");
assert.sameValue(-1 < 0, true, "-1 < 0");
assert.sameValue(0 < -1, false, "0 < -1");

// Greater than
assert.sameValue(2 > 1, true, "2 > 1");
assert.sameValue(1 > 2, false, "1 > 2");
assert.sameValue(1 > 1, false, "1 > 1");
assert.sameValue(0 > -1, true, "0 > -1");

// Less than or equal
assert.sameValue(1 <= 2, true, "1 <= 2");
assert.sameValue(2 <= 1, false, "2 <= 1");
assert.sameValue(1 <= 1, true, "1 <= 1");

// Greater than or equal
assert.sameValue(2 >= 1, true, "2 >= 1");
assert.sameValue(1 >= 2, false, "1 >= 2");
assert.sameValue(1 >= 1, true, "1 >= 1");

// Float comparison
assert.sameValue(1.5 < 2.5, true, "float comparison");
assert.sameValue(2.5 > 1.5, true, "float comparison");

// String comparison (lexicographic)
assert.sameValue("a" < "b", true, "string a < b");
assert.sameValue("b" > "a", true, "string b > a");
assert.sameValue("abc" < "abd", true, "string abc < abd");
assert.sameValue("10" < "9", true, "string 10 < 9 (lexicographic)");

// Mixed type comparison
assert.sameValue(1 < "2", true, "number < string number");
assert.sameValue("1" < 2, true, "string number < number");

printTestResults();
