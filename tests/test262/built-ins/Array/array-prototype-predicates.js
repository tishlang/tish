// Test262: Array prototype predicate methods

// some
let arr = [1, 2, 3, 4, 5];
assert.sameValue(arr.some(x => x > 3), true, "some found");
assert.sameValue(arr.some(x => x > 10), false, "some not found");
assert.sameValue(arr.some(x => x === 1), true, "some first element");
assert.sameValue(arr.some(x => x === 5), true, "some last element");

// some with empty array
assert.sameValue([].some(x => true), false, "some empty array");

// some short-circuits
let callCount = 0;
arr.some(x => {
    callCount += 1;
    return x === 2;
});
assert.sameValue(callCount, 2, "some short-circuits");

// every
arr = [2, 4, 6, 8, 10];
assert.sameValue(arr.every(x => x % 2 === 0), true, "every all match");
assert.sameValue(arr.every(x => x > 5), false, "every some don't match");

arr = [1, 2, 3, 4, 5];
assert.sameValue(arr.every(x => x > 0), true, "every all positive");
assert.sameValue(arr.every(x => x > 3), false, "every not all > 3");

// every with empty array
assert.sameValue([].every(x => false), true, "every empty array (vacuous truth)");

// every short-circuits
callCount = 0;
arr.every(x => {
    callCount += 1;
    return x < 3;
});
assert.sameValue(callCount, 3, "every short-circuits");

// flat - single level
arr = [1, [2, 3], [4, 5]];
let flattened = arr.flat();
assert.sameValue(flattened.length, 5, "flat single level length");
assert.sameValue(flattened[0], 1, "flat[0]");
assert.sameValue(flattened[1], 2, "flat[1]");
assert.sameValue(flattened[4], 5, "flat[4]");

// flat - already flat
arr = [1, 2, 3];
flattened = arr.flat();
assert.sameValue(flattened.length, 3, "flat already flat");

// flat - mixed nesting
arr = [1, [2], [3, 4], 5];
flattened = arr.flat();
assert.sameValue(flattened.length, 5, "flat mixed");
assert.sameValue(flattened[2], 3, "flat mixed[2]");

// flat - empty arrays
arr = [1, [], 2, [], 3];
flattened = arr.flat();
assert.sameValue(flattened.length, 3, "flat removes empty");
assert.sameValue(flattened[1], 2, "flat removes empty[1]");

// Combining predicates
let numbers = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10];

// Check if any even number > 5
assert.sameValue(numbers.some(x => x % 2 === 0 && x > 5), true, "some with compound");

// Check if all positives
assert.sameValue(numbers.every(x => x > 0), true, "every positive");

// Filter then some
let evens = numbers.filter(x => x % 2 === 0);
assert.sameValue(evens.some(x => x > 5), true, "filter then some");

// Map then every
let doubled = numbers.map(x => x * 2);
assert.sameValue(doubled.every(x => x % 2 === 0), true, "map then every");

printTestResults();
