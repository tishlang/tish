// Test262: Array prototype iterator methods

// map
let arr = [1, 2, 3, 4, 5];
let mapped = arr.map(x => x * 2);
assert.sameValue(mapped.length, 5, "map preserves length");
assert.sameValue(mapped[0], 2, "map[0]");
assert.sameValue(mapped[4], 10, "map[4]");
assert.sameValue(arr[0], 1, "map doesn't modify original");

// map with index
mapped = arr.map((x, i) => x + i);
assert.sameValue(mapped[0], 1, "map with index[0]");
assert.sameValue(mapped[2], 5, "map with index[2]");

// filter
arr = [1, 2, 3, 4, 5, 6];
let filtered = arr.filter(x => x % 2 === 0);
assert.sameValue(filtered.length, 3, "filter even numbers");
assert.sameValue(filtered[0], 2, "filtered[0]");
assert.sameValue(filtered[1], 4, "filtered[1]");
assert.sameValue(filtered[2], 6, "filtered[2]");

// filter with index
filtered = arr.filter((x, i) => i < 3);
assert.sameValue(filtered.length, 3, "filter by index");
assert.sameValue(filtered[2], 3, "filter by index[2]");

// filter empty result
filtered = arr.filter(x => x > 100);
assert.sameValue(filtered.length, 0, "filter no matches");

// reduce
arr = [1, 2, 3, 4, 5];
let sum = arr.reduce((acc, x) => acc + x, 0);
assert.sameValue(sum, 15, "reduce sum");

let product = arr.reduce((acc, x) => acc * x, 1);
assert.sameValue(product, 120, "reduce product");

// reduce without initial value
sum = arr.reduce((acc, x) => acc + x);
assert.sameValue(sum, 15, "reduce without initial");

// reduce to build object
let words = ["apple", "banana", "cherry"];
let lengths = words.reduce((acc, word) => {
    acc[word] = word.length;
    return acc;
}, {});
assert.sameValue(lengths.apple, 5, "reduce to object");
assert.sameValue(lengths.banana, 6, "reduce to object 2");

// forEach
arr = [1, 2, 3];
let collected = [];
arr.forEach(x => collected.push(x * 10));
assert.sameValue(collected.length, 3, "forEach collected");
assert.sameValue(collected[0], 10, "forEach[0]");
assert.sameValue(collected[2], 30, "forEach[2]");

// forEach with index
collected = [];
arr.forEach((x, i) => collected.push(i));
assert.sameValue(collected[0], 0, "forEach index[0]");
assert.sameValue(collected[2], 2, "forEach index[2]");

// find
arr = [1, 2, 3, 4, 5];
let found = arr.find(x => x > 3);
assert.sameValue(found, 4, "find first match");

found = arr.find(x => x > 10);
assert.sameValue(found, null, "find no match");

// find with objects
let people = [
    { name: "Alice", age: 30 },
    { name: "Bob", age: 25 },
    { name: "Charlie", age: 35 }
];
let person = people.find(p => p.age > 28);
assert.sameValue(person.name, "Alice", "find object");

// findIndex
arr = [1, 2, 3, 4, 5];
let idx = arr.findIndex(x => x > 3);
assert.sameValue(idx, 3, "findIndex found");

idx = arr.findIndex(x => x > 10);
assert.sameValue(idx, -1, "findIndex not found");

printTestResults();
