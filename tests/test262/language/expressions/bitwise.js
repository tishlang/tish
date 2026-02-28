// Test262: Bitwise operators (&, |, ^, ~, <<, >>)

// Bitwise AND (&)
assert.sameValue(5 & 3, 1, "5 & 3 = 1 (0101 & 0011 = 0001)");
assert.sameValue(12 & 10, 8, "12 & 10 = 8 (1100 & 1010 = 1000)");
assert.sameValue(0 & 255, 0, "0 & anything = 0");
assert.sameValue(255 & 255, 255, "255 & 255 = 255");

// Bitwise OR (|)
assert.sameValue(5 | 3, 7, "5 | 3 = 7 (0101 | 0011 = 0111)");
assert.sameValue(12 | 10, 14, "12 | 10 = 14 (1100 | 1010 = 1110)");
assert.sameValue(0 | 255, 255, "0 | 255 = 255");
assert.sameValue(0 | 0, 0, "0 | 0 = 0");

// Bitwise XOR (^)
assert.sameValue(5 ^ 3, 6, "5 ^ 3 = 6 (0101 ^ 0011 = 0110)");
assert.sameValue(12 ^ 10, 6, "12 ^ 10 = 6 (1100 ^ 1010 = 0110)");
assert.sameValue(5 ^ 5, 0, "x ^ x = 0");
assert.sameValue(0 ^ 5, 5, "0 ^ x = x");

// Bitwise NOT (~)
assert.sameValue(~0, -1, "~0 = -1");
assert.sameValue(~1, -2, "~1 = -2");
assert.sameValue(~-1, 0, "~-1 = 0");
assert.sameValue(~~5, 5, "~~x = x");

// Left shift (<<)
assert.sameValue(1 << 0, 1, "1 << 0 = 1");
assert.sameValue(1 << 1, 2, "1 << 1 = 2");
assert.sameValue(1 << 2, 4, "1 << 2 = 4");
assert.sameValue(1 << 3, 8, "1 << 3 = 8");
assert.sameValue(5 << 2, 20, "5 << 2 = 20");

// Right shift (>>)
assert.sameValue(8 >> 0, 8, "8 >> 0 = 8");
assert.sameValue(8 >> 1, 4, "8 >> 1 = 4");
assert.sameValue(8 >> 2, 2, "8 >> 2 = 2");
assert.sameValue(8 >> 3, 1, "8 >> 3 = 1");
assert.sameValue(20 >> 2, 5, "20 >> 2 = 5");

// Negative right shift
assert.sameValue(-8 >> 1, -4, "-8 >> 1 = -4 (sign-extending)");

printTestResults();
