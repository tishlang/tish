// Test262: Nullish coalescing operator (??)

// Basic nullish coalescing
assert.sameValue(null ?? "default", "default", "null ?? default");
assert.sameValue("value" ?? "default", "value", "value ?? default");

// Difference from || operator
assert.sameValue(0 ?? "default", 0, "0 ?? default (0 is not nullish)");
assert.sameValue(0 || "default", "default", "0 || default (0 is falsy)");

assert.sameValue("" ?? "default", "", "'' ?? default ('' is not nullish)");
assert.sameValue("" || "default", "default", "'' || default ('' is falsy)");

assert.sameValue(false ?? "default", false, "false ?? default (false is not nullish)");
assert.sameValue(false || "default", "default", "false || default (false is falsy)");

// Chained nullish coalescing
assert.sameValue(null ?? null ?? "final", "final", "chained with multiple nulls");
assert.sameValue(null ?? "first" ?? "second", "first", "chained stops at first non-nullish");

// With variables
let a = null;
let b = "value";
assert.sameValue(a ?? b, "value", "variable nullish coalescing");

let c = 0;
assert.sameValue(c ?? 10, 0, "zero is not nullish");

// With object properties
let obj = { x: null, y: 5 };
assert.sameValue(obj.x ?? 10, 10, "null property ?? default");
assert.sameValue(obj.y ?? 10, 5, "defined property ?? default");
assert.sameValue(obj.z ?? 10, 10, "missing property ?? default");

// Combined with optional chaining
let nested = { a: { b: null } };
assert.sameValue(nested?.a?.b ?? "default", "default", "optional chaining with nullish coalescing");
assert.sameValue(nested?.a?.c ?? "default", "default", "missing property with nullish coalescing");
assert.sameValue(nested?.x?.y ?? "default", "default", "missing chain with nullish coalescing");

printTestResults();
