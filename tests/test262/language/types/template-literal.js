// Test262: Template literals

// Basic template literal
assert.sameValue(`hello`, "hello", "basic template");
assert.sameValue(``, "", "empty template");

// Template with interpolation
let name = "World";
assert.sameValue(`Hello, ${name}!`, "Hello, World!", "simple interpolation");

// Multiple interpolations
let a = 1;
let b = 2;
assert.sameValue(`${a} + ${b} = ${a + b}`, "1 + 2 = 3", "multiple interpolations");

// Expression in interpolation
assert.sameValue(`2 + 3 = ${2 + 3}`, "2 + 3 = 5", "expression interpolation");
assert.sameValue(`${10 * 5}`, "50", "arithmetic in template");

// Nested expressions
let arr = [1, 2, 3];
assert.sameValue(`length: ${arr.length}`, "length: 3", "property access in template");

// Object property in template
let obj = { x: 10, y: 20 };
assert.sameValue(`x=${obj.x}, y=${obj.y}`, "x=10, y=20", "object properties in template");

// Function call in template
function double(n) { return n * 2; }
assert.sameValue(`double(5) = ${double(5)}`, "double(5) = 10", "function call in template");

// Conditional in template
let val = 5;
assert.sameValue(`${val > 0 ? "positive" : "non-positive"}`, "positive", "ternary in template");

// Nested template (if supported)
let inner = "inner";
assert.sameValue(`outer ${`nested ${inner}`}`, "outer nested inner", "nested template");

// Template with special characters
assert.sameValue(`line1\nline2`, "line1\nline2", "newline in template");
assert.sameValue(`tab\there`, "tab\there", "tab in template");

// Template with backtick escape
assert.sameValue(`back\`tick`, "back`tick", "escaped backtick");

// Template with dollar sign
assert.sameValue(`price: \$100`, "price: $100", "escaped dollar sign");

// Boolean interpolation
assert.sameValue(`${true}`, "true", "true interpolation");
assert.sameValue(`${false}`, "false", "false interpolation");

// Null interpolation
assert.sameValue(`${null}`, "null", "null interpolation");

// Number interpolation
assert.sameValue(`${42}`, "42", "number interpolation");
assert.sameValue(`${3.14}`, "3.14", "float interpolation");

// Array interpolation
assert.sameValue(`${[1,2,3]}`, "1,2,3", "array interpolation");

printTestResults();
