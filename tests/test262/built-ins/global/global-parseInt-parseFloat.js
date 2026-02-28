// Test262: parseInt, parseFloat, isFinite, isNaN

// parseInt - basic
assert.sameValue(parseInt("42"), 42, "parseInt integer string");
assert.sameValue(parseInt("123"), 123, "parseInt larger");
assert.sameValue(parseInt("-42"), -42, "parseInt negative");
assert.sameValue(parseInt("0"), 0, "parseInt zero");

// parseInt - with leading/trailing
assert.sameValue(parseInt("  42  "), 42, "parseInt with spaces");
assert.sameValue(parseInt("42abc"), 42, "parseInt with trailing text");
assert.sameValue(parseInt("42.9"), 42, "parseInt truncates decimal");

// parseInt - non-numeric
assert.sameValue(isNaN(parseInt("abc")), true, "parseInt non-numeric is NaN");
assert.sameValue(isNaN(parseInt("")), true, "parseInt empty is NaN");

// parseInt - with radix
assert.sameValue(parseInt("10", 2), 2, "parseInt binary");
assert.sameValue(parseInt("10", 8), 8, "parseInt octal");
assert.sameValue(parseInt("10", 10), 10, "parseInt decimal");
assert.sameValue(parseInt("10", 16), 16, "parseInt hex");
assert.sameValue(parseInt("ff", 16), 255, "parseInt hex ff");
assert.sameValue(parseInt("FF", 16), 255, "parseInt hex FF");

// parseFloat - basic
assert.sameValue(parseFloat("3.14"), 3.14, "parseFloat");
assert.sameValue(parseFloat("42"), 42, "parseFloat integer");
assert.sameValue(parseFloat("-3.14"), -3.14, "parseFloat negative");
assert.sameValue(parseFloat("0.5"), 0.5, "parseFloat decimal");

// parseFloat - with text
assert.sameValue(parseFloat("  3.14  "), 3.14, "parseFloat with spaces");
assert.sameValue(parseFloat("3.14abc"), 3.14, "parseFloat with trailing");
assert.sameValue(parseFloat(".5"), 0.5, "parseFloat leading dot");

// parseFloat - non-numeric
assert.sameValue(isNaN(parseFloat("abc")), true, "parseFloat non-numeric is NaN");
assert.sameValue(isNaN(parseFloat("")), true, "parseFloat empty is NaN");

// parseFloat - scientific notation
assert.sameValue(parseFloat("1e2"), 100, "parseFloat scientific");
assert.sameValue(parseFloat("1.5e3"), 1500, "parseFloat scientific with decimal");

// isFinite
assert.sameValue(isFinite(42), true, "isFinite number");
assert.sameValue(isFinite(3.14), true, "isFinite float");
assert.sameValue(isFinite(-100), true, "isFinite negative");
assert.sameValue(isFinite(0), true, "isFinite zero");

assert.sameValue(isFinite(Infinity), false, "isFinite Infinity");
assert.sameValue(isFinite(-Infinity), false, "isFinite -Infinity");
assert.sameValue(isFinite(NaN), false, "isFinite NaN");

// isFinite with coercion
assert.sameValue(isFinite("42"), true, "isFinite string number");
assert.sameValue(isFinite(""), true, "isFinite empty string (coerces to 0)");
assert.sameValue(isFinite("abc"), false, "isFinite non-numeric string");
assert.sameValue(isFinite(null), true, "isFinite null (coerces to 0)");

// isNaN
assert.sameValue(isNaN(NaN), true, "isNaN NaN");
assert.sameValue(isNaN(42), false, "isNaN number");
assert.sameValue(isNaN(3.14), false, "isNaN float");
assert.sameValue(isNaN(Infinity), false, "isNaN Infinity");

// isNaN with coercion
assert.sameValue(isNaN("abc"), true, "isNaN non-numeric string");
assert.sameValue(isNaN("42"), false, "isNaN numeric string");
assert.sameValue(isNaN(""), false, "isNaN empty string (coerces to 0)");
assert.sameValue(isNaN(null), false, "isNaN null (coerces to 0)");
assert.sameValue(isNaN(true), false, "isNaN true (coerces to 1)");
assert.sameValue(isNaN(false), false, "isNaN false (coerces to 0)");

// Practical usage
function safeParseInt(str, defaultValue) {
    let result = parseInt(str);
    return isNaN(result) ? defaultValue : result;
}
assert.sameValue(safeParseInt("42", 0), 42, "safeParseInt valid");
assert.sameValue(safeParseInt("abc", 0), 0, "safeParseInt invalid");

printTestResults();
