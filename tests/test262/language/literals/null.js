// Test262: null literal

// null value
assert.sameValue(null, null, "null literal");

// null equality
assert.sameValue(null === null, true, "null === null");
assert.sameValue(null == null, true, "null == null");

// null is falsy
let result = "initial";
if (null) {
    result = "truthy";
} else {
    result = "falsy";
}
assert.sameValue(result, "falsy", "null is falsy");

// null with logical NOT
assert.sameValue(!null, true, "!null is true");
assert.sameValue(!!null, false, "!!null is false");

// null comparisons
assert.sameValue(null === false, false, "null !== false");
assert.sameValue(null === 0, false, "null !== 0");
assert.sameValue(null === "", false, "null !== empty string");

// null with nullish coalescing
assert.sameValue(null ?? "default", "default", "null ?? default");
assert.sameValue(null ?? 0, 0, "null ?? 0");
assert.sameValue(null ?? false, false, "null ?? false");

// null with logical OR
assert.sameValue(null || "fallback", "fallback", "null || fallback");
assert.sameValue(null || 0, 0, "null || 0");

// null with logical AND
assert.sameValue(null && "value", null, "null && value");

// typeof null (historical quirk)
assert.sameValue(typeof null, "object", "typeof null is 'object'");

// null in object
let obj = { value: null };
assert.sameValue(obj.value, null, "object property is null");
assert.sameValue(obj.value === null, true, "check property is null");

// null in array
let arr = [1, null, 3];
assert.sameValue(arr[1], null, "array element is null");
assert.sameValue(arr[1] === null, true, "check array element is null");

// null return
function returnNull() {
    return null;
}
assert.sameValue(returnNull(), null, "function returns null");

// null parameter
function acceptNull(x) {
    return x === null ? "was null" : "was not null";
}
assert.sameValue(acceptNull(null), "was null", "pass null as argument");
assert.sameValue(acceptNull(0), "was not null", "pass 0 as argument");

printTestResults();
