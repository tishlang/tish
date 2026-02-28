// Test262: Unary operators (typeof, void, +, -, !, ~)

// Unary plus (+)
assert.sameValue(+5, 5, "+5");
assert.sameValue(+-5, -5, "+-5");
assert.sameValue(+"10", 10, "+'10' converts to number");
assert.sameValue(+true, 1, "+true = 1");
assert.sameValue(+false, 0, "+false = 0");
assert.sameValue(+null, 0, "+null = 0");

// Unary minus (-)
assert.sameValue(-5, -5, "-5");
assert.sameValue(-(-5), 5, "-(-5)");
assert.sameValue(-0, 0, "-0");
assert.sameValue(-"10", -10, "-'10' converts to number");
assert.sameValue(-true, -1, "-true = -1");

// typeof operator
assert.sameValue(typeof 42, "number", "typeof number");
assert.sameValue(typeof 3.14, "number", "typeof float");
assert.sameValue(typeof "hello", "string", "typeof string");
assert.sameValue(typeof true, "boolean", "typeof true");
assert.sameValue(typeof false, "boolean", "typeof false");
assert.sameValue(typeof null, "object", "typeof null (quirk)");
assert.sameValue(typeof [1, 2, 3], "object", "typeof array");
assert.sameValue(typeof { x: 1 }, "object", "typeof object");

// typeof with function
let fn = function() { return 1; };
assert.sameValue(typeof fn, "function", "typeof function");

let arrow = () => 1;
assert.sameValue(typeof arrow, "function", "typeof arrow function");

// void operator
assert.sameValue(void 0, null, "void 0 returns null");
assert.sameValue(void 1, null, "void 1 returns null");
assert.sameValue(void "hello", null, "void string returns null");
assert.sameValue(void (1 + 2), null, "void expression returns null");

// Logical NOT (!) - also tested in logical.js
assert.sameValue(!true, false, "!true");
assert.sameValue(!false, true, "!false");
assert.sameValue(!0, true, "!0");
assert.sameValue(!1, false, "!1");

// Bitwise NOT (~) - also tested in bitwise.js
assert.sameValue(~0, -1, "~0");
assert.sameValue(~-1, 0, "~-1");

printTestResults();
