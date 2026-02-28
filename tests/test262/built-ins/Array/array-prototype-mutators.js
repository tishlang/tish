// Test262: Array prototype mutator methods

// push
let arr = [1, 2, 3];
let newLen = arr.push(4);
assert.sameValue(newLen, 4, "push returns new length");
assert.sameValue(arr.length, 4, "push increases length");
assert.sameValue(arr[3], 4, "push adds element");

arr.push(5, 6);
assert.sameValue(arr.length, 6, "push multiple elements");
assert.sameValue(arr[5], 6, "push multiple - last element");

// pop
arr = [1, 2, 3];
let popped = arr.pop();
assert.sameValue(popped, 3, "pop returns last element");
assert.sameValue(arr.length, 2, "pop decreases length");

popped = arr.pop();
assert.sameValue(popped, 2, "pop second element");

// shift
arr = [1, 2, 3];
let shifted = arr.shift();
assert.sameValue(shifted, 1, "shift returns first element");
assert.sameValue(arr.length, 2, "shift decreases length");
assert.sameValue(arr[0], 2, "shift moves elements");

// unshift
arr = [2, 3];
newLen = arr.unshift(1);
assert.sameValue(newLen, 3, "unshift returns new length");
assert.sameValue(arr[0], 1, "unshift adds to front");
assert.sameValue(arr[1], 2, "unshift shifts existing");

arr.unshift(-1, 0);
assert.sameValue(arr.length, 5, "unshift multiple");
assert.sameValue(arr[0], -1, "unshift multiple - first");
assert.sameValue(arr[1], 0, "unshift multiple - second");

// splice - remove
arr = [1, 2, 3, 4, 5];
let removed = arr.splice(2, 2);
assert.sameValue(removed.length, 2, "splice remove returns removed");
assert.sameValue(removed[0], 3, "splice removed[0]");
assert.sameValue(removed[1], 4, "splice removed[1]");
assert.sameValue(arr.length, 3, "splice decreases length");
assert.sameValue(arr[2], 5, "splice shifts remaining");

// splice - insert
arr = [1, 2, 5];
arr.splice(2, 0, 3, 4);
assert.sameValue(arr.length, 5, "splice insert increases length");
assert.sameValue(arr[2], 3, "splice inserted[0]");
assert.sameValue(arr[3], 4, "splice inserted[1]");
assert.sameValue(arr[4], 5, "splice shifted element");

// splice - replace
arr = [1, 2, 3, 4, 5];
removed = arr.splice(1, 2, 20, 30);
assert.sameValue(removed.length, 2, "splice replace - removed");
assert.sameValue(arr[1], 20, "splice replace - new[0]");
assert.sameValue(arr[2], 30, "splice replace - new[1]");

// reverse
arr = [1, 2, 3, 4, 5];
let reversed = arr.reverse();
assert.sameValue(arr[0], 5, "reverse first");
assert.sameValue(arr[4], 1, "reverse last");
assert.sameValue(reversed, arr, "reverse returns same array");

// sort - default (lexicographic)
arr = [3, 1, 4, 1, 5, 9, 2, 6];
arr.sort();
assert.sameValue(arr[0], 1, "sort[0]");
assert.sameValue(arr[1], 1, "sort[1]");
assert.sameValue(arr[7], 9, "sort[7]");

// sort - numeric
arr = [10, 2, 30, 4];
arr.sort((a, b) => a - b);
assert.sameValue(arr[0], 2, "numeric sort[0]");
assert.sameValue(arr[1], 4, "numeric sort[1]");
assert.sameValue(arr[2], 10, "numeric sort[2]");
assert.sameValue(arr[3], 30, "numeric sort[3]");

// sort - descending
arr = [1, 5, 3, 2, 4];
arr.sort((a, b) => b - a);
assert.sameValue(arr[0], 5, "descending sort[0]");
assert.sameValue(arr[4], 1, "descending sort[4]");

printTestResults();
