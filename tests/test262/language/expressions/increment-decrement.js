// Test262: Increment (++) and Decrement (--) operators

// Prefix increment
let x = 5;
assert.sameValue(++x, 6, "prefix increment returns new value");
assert.sameValue(x, 6, "variable is incremented");

// Postfix increment
x = 5;
assert.sameValue(x++, 5, "postfix increment returns old value");
assert.sameValue(x, 6, "variable is incremented after");

// Prefix decrement
x = 5;
assert.sameValue(--x, 4, "prefix decrement returns new value");
assert.sameValue(x, 4, "variable is decremented");

// Postfix decrement
x = 5;
assert.sameValue(x--, 5, "postfix decrement returns old value");
assert.sameValue(x, 4, "variable is decremented after");

// Multiple increments
x = 0;
x++;
x++;
x++;
assert.sameValue(x, 3, "multiple increments");

// Multiple decrements
x = 10;
x--;
x--;
x--;
assert.sameValue(x, 7, "multiple decrements");

// In expressions
x = 5;
let y = x++ + 10;
assert.sameValue(y, 15, "postfix in expression uses old value");
assert.sameValue(x, 6, "variable incremented after expression");

x = 5;
y = ++x + 10;
assert.sameValue(y, 16, "prefix in expression uses new value");
assert.sameValue(x, 6, "variable incremented before expression");

// Negative numbers
x = -5;
assert.sameValue(++x, -4, "increment negative number");
x = -5;
assert.sameValue(--x, -6, "decrement negative number");

// Zero crossing
x = 1;
x--;
x--;
assert.sameValue(x, -1, "decrement crosses zero");

x = -1;
x++;
x++;
assert.sameValue(x, 1, "increment crosses zero");

// Float increment
x = 1.5;
x++;
assert.sameValue(x, 2.5, "float increment");

x = 1.5;
x--;
assert.sameValue(x, 0.5, "float decrement");

printTestResults();
