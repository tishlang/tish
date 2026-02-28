// Test262: Object.assign and Object.fromEntries

// Object.assign - basic
let target = { a: 1 };
let source = { b: 2 };
let result = Object.assign(target, source);
assert.sameValue(result.a, 1, "assign keeps target props");
assert.sameValue(result.b, 2, "assign adds source props");
assert.sameValue(target.b, 2, "assign modifies target");

// Object.assign - multiple sources
target = {};
result = Object.assign(target, { a: 1 }, { b: 2 }, { c: 3 });
assert.sameValue(result.a, 1, "multiple sources a");
assert.sameValue(result.b, 2, "multiple sources b");
assert.sameValue(result.c, 3, "multiple sources c");

// Object.assign - override
target = { a: 1, b: 2 };
source = { b: 20, c: 30 };
result = Object.assign(target, source);
assert.sameValue(result.a, 1, "override keeps non-conflicting");
assert.sameValue(result.b, 20, "override replaces conflicting");
assert.sameValue(result.c, 30, "override adds new");

// Object.assign - copy
let original = { x: 10, y: 20 };
let copy = Object.assign({}, original);
assert.sameValue(copy.x, 10, "copy x");
assert.sameValue(copy.y, 20, "copy y");
copy.x = 100;
assert.sameValue(original.x, 10, "copy is independent");

// Object.assign - returns target
target = { a: 1 };
result = Object.assign(target, { b: 2 });
assert.sameValue(result === target, true, "assign returns target");

// Object.assign - empty source
target = { a: 1 };
result = Object.assign(target, {});
assert.sameValue(result.a, 1, "empty source unchanged");

// Object.fromEntries - basic
let entries = [["a", 1], ["b", 2], ["c", 3]];
let obj = Object.fromEntries(entries);
assert.sameValue(obj.a, 1, "fromEntries a");
assert.sameValue(obj.b, 2, "fromEntries b");
assert.sameValue(obj.c, 3, "fromEntries c");

// Object.fromEntries - empty
obj = Object.fromEntries([]);
assert.sameValue(Object.keys(obj).length, 0, "fromEntries empty");

// Object.fromEntries - single entry
obj = Object.fromEntries([["only", "one"]]);
assert.sameValue(obj.only, "one", "fromEntries single");

// Round-trip: entries -> fromEntries
original = { name: "Alice", age: 30 };
entries = Object.entries(original);
let reconstructed = Object.fromEntries(entries);
assert.sameValue(reconstructed.name, "Alice", "round-trip name");
assert.sameValue(reconstructed.age, 30, "round-trip age");

// Transform object via entries
obj = { a: 1, b: 2, c: 3 };
let doubled = Object.fromEntries(
    Object.entries(obj).map(([k, v]) => [k, v * 2])
);
assert.sameValue(doubled.a, 2, "transform via entries a");
assert.sameValue(doubled.b, 4, "transform via entries b");
assert.sameValue(doubled.c, 6, "transform via entries c");

// Filter object via entries
obj = { a: 1, b: 2, c: 3, d: 4 };
let filtered = Object.fromEntries(
    Object.entries(obj).filter(([k, v]) => v > 2)
);
assert.sameValue(filtered.a, null, "filtered out a");
assert.sameValue(filtered.b, null, "filtered out b");
assert.sameValue(filtered.c, 3, "filtered kept c");
assert.sameValue(filtered.d, 4, "filtered kept d");

// Rename keys
obj = { oldName: "value" };
let renamed = Object.fromEntries(
    Object.entries(obj).map(([k, v]) => [k === "oldName" ? "newName" : k, v])
);
assert.sameValue(renamed.newName, "value", "renamed key");
assert.sameValue(renamed.oldName, null, "old key gone");

printTestResults();
