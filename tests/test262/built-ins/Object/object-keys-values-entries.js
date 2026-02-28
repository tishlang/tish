// Test262: Object.keys, Object.values, Object.entries

// Object.keys
let obj = { a: 1, b: 2, c: 3 };
let keys = Object.keys(obj);
assert.sameValue(keys.length, 3, "keys length");
assert.sameValue(keys.includes("a"), true, "keys has a");
assert.sameValue(keys.includes("b"), true, "keys has b");
assert.sameValue(keys.includes("c"), true, "keys has c");

// Object.keys - empty object
keys = Object.keys({});
assert.sameValue(keys.length, 0, "empty object keys");

// Object.keys - order
obj = { z: 1, a: 2, m: 3 };
keys = Object.keys(obj);
assert.sameValue(keys.length, 3, "keys preserves all");

// Object.values
obj = { a: 1, b: 2, c: 3 };
let values = Object.values(obj);
assert.sameValue(values.length, 3, "values length");
assert.sameValue(values.includes(1), true, "values has 1");
assert.sameValue(values.includes(2), true, "values has 2");
assert.sameValue(values.includes(3), true, "values has 3");

// Object.values - different types
obj = { num: 42, str: "hello", bool: true };
values = Object.values(obj);
assert.sameValue(values.length, 3, "mixed values length");
assert.sameValue(values.includes(42), true, "values has number");
assert.sameValue(values.includes("hello"), true, "values has string");
assert.sameValue(values.includes(true), true, "values has boolean");

// Object.values - empty object
values = Object.values({});
assert.sameValue(values.length, 0, "empty object values");

// Object.entries
obj = { a: 1, b: 2 };
let entries = Object.entries(obj);
assert.sameValue(entries.length, 2, "entries length");
assert.sameValue(Array.isArray(entries[0]), true, "entry is array");
assert.sameValue(entries[0].length, 2, "entry has 2 elements");

// Object.entries - access key/value
let found = entries.find(e => e[0] === "a");
assert.sameValue(found[1], 1, "entries a value");

found = entries.find(e => e[0] === "b");
assert.sameValue(found[1], 2, "entries b value");

// Object.entries - empty object
entries = Object.entries({});
assert.sameValue(entries.length, 0, "empty object entries");

// Iterate with Object.keys
obj = { x: 10, y: 20, z: 30 };
let sum = 0;
for (let key of Object.keys(obj)) {
    sum += obj[key];
}
assert.sameValue(sum, 60, "iterate keys sum values");

// Iterate with Object.values
sum = 0;
for (let value of Object.values(obj)) {
    sum += value;
}
assert.sameValue(sum, 60, "iterate values directly");

// Iterate with Object.entries
sum = 0;
for (let [key, value] of Object.entries(obj)) {
    sum += value;
}
assert.sameValue(sum, 60, "iterate entries destructured");

// Transform with Object.entries
obj = { a: 1, b: 2, c: 3 };
let doubled = {};
for (let [k, v] of Object.entries(obj)) {
    doubled[k] = v * 2;
}
assert.sameValue(doubled.a, 2, "transform a");
assert.sameValue(doubled.b, 4, "transform b");
assert.sameValue(doubled.c, 6, "transform c");

printTestResults();
