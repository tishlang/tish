// Test262: if/else statements

// Basic if
let result = "";
if (true) {
    result = "executed";
}
assert.sameValue(result, "executed", "if true executes body");

result = "initial";
if (false) {
    result = "executed";
}
assert.sameValue(result, "initial", "if false skips body");

// if/else
if (true) {
    result = "if branch";
} else {
    result = "else branch";
}
assert.sameValue(result, "if branch", "if/else takes if branch when true");

if (false) {
    result = "if branch";
} else {
    result = "else branch";
}
assert.sameValue(result, "else branch", "if/else takes else branch when false");

// if/else if/else
let x = 5;
if (x < 0) {
    result = "negative";
} else if (x === 0) {
    result = "zero";
} else if (x < 10) {
    result = "small";
} else {
    result = "large";
}
assert.sameValue(result, "small", "else if chain");

x = -5;
if (x < 0) {
    result = "negative";
} else if (x === 0) {
    result = "zero";
} else {
    result = "positive";
}
assert.sameValue(result, "negative", "else if - first condition");

x = 0;
if (x < 0) {
    result = "negative";
} else if (x === 0) {
    result = "zero";
} else {
    result = "positive";
}
assert.sameValue(result, "zero", "else if - second condition");

// Nested if
x = 10;
let y = 20;
if (x > 5) {
    if (y > 15) {
        result = "both";
    } else {
        result = "only x";
    }
} else {
    result = "neither";
}
assert.sameValue(result, "both", "nested if");

// Truthy/falsy conditions
if (1) { result = "truthy"; } else { result = "falsy"; }
assert.sameValue(result, "truthy", "1 is truthy");

if (0) { result = "truthy"; } else { result = "falsy"; }
assert.sameValue(result, "falsy", "0 is falsy");

if ("hello") { result = "truthy"; } else { result = "falsy"; }
assert.sameValue(result, "truthy", "non-empty string is truthy");

if ("") { result = "truthy"; } else { result = "falsy"; }
assert.sameValue(result, "falsy", "empty string is falsy");

if (null) { result = "truthy"; } else { result = "falsy"; }
assert.sameValue(result, "falsy", "null is falsy");

printTestResults();
