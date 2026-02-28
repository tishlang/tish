// Test262: Math functions and constants

// Math constants
assert.sameValue(typeof Math.PI, "number", "Math.PI is number");
assert.sameValue(Math.PI > 3.14, true, "Math.PI > 3.14");
assert.sameValue(Math.PI < 3.15, true, "Math.PI < 3.15");

assert.sameValue(typeof Math.E, "number", "Math.E is number");
assert.sameValue(Math.E > 2.71, true, "Math.E > 2.71");
assert.sameValue(Math.E < 2.72, true, "Math.E < 2.72");

// Math.abs
assert.sameValue(Math.abs(5), 5, "abs positive");
assert.sameValue(Math.abs(-5), 5, "abs negative");
assert.sameValue(Math.abs(0), 0, "abs zero");
assert.sameValue(Math.abs(-3.14), 3.14, "abs negative float");

// Math.sqrt
assert.sameValue(Math.sqrt(4), 2, "sqrt 4");
assert.sameValue(Math.sqrt(9), 3, "sqrt 9");
assert.sameValue(Math.sqrt(16), 4, "sqrt 16");
assert.sameValue(Math.sqrt(2) > 1.41, true, "sqrt 2 > 1.41");
assert.sameValue(Math.sqrt(2) < 1.42, true, "sqrt 2 < 1.42");
assert.sameValue(Math.sqrt(0), 0, "sqrt 0");
assert.sameValue(Math.sqrt(1), 1, "sqrt 1");

// Math.min / Math.max
assert.sameValue(Math.min(1, 2, 3), 1, "min of 1,2,3");
assert.sameValue(Math.min(3, 1, 2), 1, "min unordered");
assert.sameValue(Math.min(-5, 0, 5), -5, "min with negative");
assert.sameValue(Math.min(5), 5, "min single");

assert.sameValue(Math.max(1, 2, 3), 3, "max of 1,2,3");
assert.sameValue(Math.max(3, 1, 2), 3, "max unordered");
assert.sameValue(Math.max(-5, 0, 5), 5, "max with negative");
assert.sameValue(Math.max(5), 5, "max single");

// Math.floor / Math.ceil / Math.round
assert.sameValue(Math.floor(3.7), 3, "floor 3.7");
assert.sameValue(Math.floor(3.2), 3, "floor 3.2");
assert.sameValue(Math.floor(3), 3, "floor integer");
assert.sameValue(Math.floor(-3.2), -4, "floor negative");

assert.sameValue(Math.ceil(3.2), 4, "ceil 3.2");
assert.sameValue(Math.ceil(3.7), 4, "ceil 3.7");
assert.sameValue(Math.ceil(3), 3, "ceil integer");
assert.sameValue(Math.ceil(-3.7), -3, "ceil negative");

assert.sameValue(Math.round(3.4), 3, "round down");
assert.sameValue(Math.round(3.5), 4, "round half up");
assert.sameValue(Math.round(3.6), 4, "round up");
assert.sameValue(Math.round(-3.5), -3, "round negative");

// Math.trunc
assert.sameValue(Math.trunc(3.7), 3, "trunc positive");
assert.sameValue(Math.trunc(-3.7), -3, "trunc negative");
assert.sameValue(Math.trunc(3), 3, "trunc integer");

// Math.sign
assert.sameValue(Math.sign(5), 1, "sign positive");
assert.sameValue(Math.sign(-5), -1, "sign negative");
assert.sameValue(Math.sign(0), 0, "sign zero");

// Math.pow
assert.sameValue(Math.pow(2, 3), 8, "pow 2^3");
assert.sameValue(Math.pow(10, 2), 100, "pow 10^2");
assert.sameValue(Math.pow(2, 0), 1, "pow x^0");
assert.sameValue(Math.pow(2, -1), 0.5, "pow negative exp");

// Math.log / Math.exp
assert.sameValue(Math.exp(0), 1, "exp 0");
assert.sameValue(Math.exp(1) > 2.71, true, "exp 1 > 2.71");
assert.sameValue(Math.exp(1) < 2.72, true, "exp 1 < 2.72");

assert.sameValue(Math.log(1), 0, "log 1");
assert.sameValue(Math.log(Math.E) > 0.99, true, "log E ~= 1");
assert.sameValue(Math.log(Math.E) < 1.01, true, "log E ~= 1");

// Math.sin / Math.cos / Math.tan
assert.sameValue(Math.sin(0), 0, "sin 0");
assert.sameValue(Math.cos(0), 1, "cos 0");
assert.sameValue(Math.tan(0), 0, "tan 0");

// sin(π/2) ≈ 1
let sinHalfPi = Math.sin(Math.PI / 2);
assert.sameValue(sinHalfPi > 0.99, true, "sin(π/2) ~= 1");

// cos(π) ≈ -1
let cosPi = Math.cos(Math.PI);
assert.sameValue(cosPi < -0.99, true, "cos(π) ~= -1");

// Math.random
let r1 = Math.random();
assert.sameValue(r1 >= 0, true, "random >= 0");
assert.sameValue(r1 < 1, true, "random < 1");

let r2 = Math.random();
assert.sameValue(r2 >= 0, true, "second random >= 0");
assert.sameValue(r2 < 1, true, "second random < 1");

printTestResults();
