// Test262: Assignment operators (=, +=, -=, *=, /=, %=)

// Basic assignment
let x = 10;
assert.sameValue(x, 10, "basic assignment");

// Addition assignment
x = 10;
x += 5;
assert.sameValue(x, 15, "x += 5");

x = 10;
x += -3;
assert.sameValue(x, 7, "x += -3");

// Subtraction assignment
x = 10;
x -= 3;
assert.sameValue(x, 7, "x -= 3");

x = 10;
x -= -3;
assert.sameValue(x, 13, "x -= -3");

// Multiplication assignment
x = 10;
x *= 3;
assert.sameValue(x, 30, "x *= 3");

x = 5;
x *= 0;
assert.sameValue(x, 0, "x *= 0");

// Division assignment
x = 20;
x /= 4;
assert.sameValue(x, 5, "x /= 4");

x = 7;
x /= 2;
assert.sameValue(x, 3.5, "x /= 2 (non-integer result)");

// Modulus assignment
x = 17;
x %= 5;
assert.sameValue(x, 2, "x %= 5");

x = 10;
x %= 10;
assert.sameValue(x, 0, "x %= x = 0");

// Chained assignment
let a = 0;
let b = 0;
let c = 0;
a = b = c = 5;
assert.sameValue(a, 5, "chained assignment a");
assert.sameValue(b, 5, "chained assignment b");
assert.sameValue(c, 5, "chained assignment c");

// String assignment
let s = "hello";
s += " world";
assert.sameValue(s, "hello world", "string +=");

printTestResults();
