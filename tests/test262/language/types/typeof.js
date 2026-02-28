// Test262: typeof operator

// Number
assert.sameValue(typeof 42, "number", "typeof integer");
assert.sameValue(typeof 3.14, "number", "typeof float");
assert.sameValue(typeof -5, "number", "typeof negative");
assert.sameValue(typeof 0, "number", "typeof zero");
assert.sameValue(typeof Infinity, "number", "typeof Infinity");
assert.sameValue(typeof NaN, "number", "typeof NaN");

// String
assert.sameValue(typeof "hello", "string", "typeof string");
assert.sameValue(typeof "", "string", "typeof empty string");
assert.sameValue(typeof "123", "string", "typeof numeric string");

// Boolean
assert.sameValue(typeof true, "boolean", "typeof true");
assert.sameValue(typeof false, "boolean", "typeof false");

// Null (historical quirk)
assert.sameValue(typeof null, "object", "typeof null is 'object'");

// Object
assert.sameValue(typeof {}, "object", "typeof empty object");
assert.sameValue(typeof { x: 1 }, "object", "typeof object");

// Array (arrays are objects)
assert.sameValue(typeof [], "object", "typeof empty array");
assert.sameValue(typeof [1, 2, 3], "object", "typeof array");

// Function
function namedFunc() { return 1; }
assert.sameValue(typeof namedFunc, "function", "typeof named function");

let arrowFunc = () => 1;
assert.sameValue(typeof arrowFunc, "function", "typeof arrow function");

let funcExpr = function() { return 1; };
assert.sameValue(typeof funcExpr, "function", "typeof function expression");

// typeof in expressions
let x = 42;
assert.sameValue(typeof x, "number", "typeof variable");

x = "hello";
assert.sameValue(typeof x, "string", "typeof reassigned variable");

// typeof with expressions
assert.sameValue(typeof (1 + 2), "number", "typeof expression result");
assert.sameValue(typeof ("a" + "b"), "string", "typeof concatenation");

// typeof comparison
assert.sameValue(typeof 42 === "number", true, "typeof comparison true");
assert.sameValue(typeof 42 === "string", false, "typeof comparison false");

// Conditional based on typeof
function describe(val) {
    if (typeof val === "number") {
        return "number";
    } else if (typeof val === "string") {
        return "string";
    } else if (typeof val === "boolean") {
        return "boolean";
    } else if (typeof val === "function") {
        return "function";
    } else {
        return "other";
    }
}
assert.sameValue(describe(42), "number", "describe number");
assert.sameValue(describe("hi"), "string", "describe string");
assert.sameValue(describe(true), "boolean", "describe boolean");
assert.sameValue(describe(() => 1), "function", "describe function");
assert.sameValue(describe({}), "other", "describe object");

printTestResults();
