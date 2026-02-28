// Test262: Destructuring assignment

// Array destructuring - basic
let [a, b, c] = [1, 2, 3];
assert.sameValue(a, 1, "array destruct a");
assert.sameValue(b, 2, "array destruct b");
assert.sameValue(c, 3, "array destruct c");

// Array destructuring - fewer variables
let [x, y] = [1, 2, 3, 4];
assert.sameValue(x, 1, "fewer vars x");
assert.sameValue(y, 2, "fewer vars y");

// Array destructuring - more variables
let [p, q, r, s] = [1, 2];
assert.sameValue(p, 1, "more vars p");
assert.sameValue(q, 2, "more vars q");
assert.sameValue(r, null, "more vars r is null");
assert.sameValue(s, null, "more vars s is null");

// Array destructuring - skip elements
let [first, , third] = [1, 2, 3];
assert.sameValue(first, 1, "skip - first");
assert.sameValue(third, 3, "skip - third");

// Array destructuring - rest
let [head, ...tail] = [1, 2, 3, 4, 5];
assert.sameValue(head, 1, "rest - head");
assert.sameValue(tail.length, 4, "rest - tail length");
assert.sameValue(tail[0], 2, "rest - tail[0]");
assert.sameValue(tail[3], 5, "rest - tail[3]");

// Array destructuring - nested
let [[inner1, inner2], outer] = [[1, 2], 3];
assert.sameValue(inner1, 1, "nested inner1");
assert.sameValue(inner2, 2, "nested inner2");
assert.sameValue(outer, 3, "nested outer");

// Object destructuring - basic
let { name, age } = { name: "Alice", age: 30 };
assert.sameValue(name, "Alice", "object destruct name");
assert.sameValue(age, 30, "object destruct age");

// Object destructuring - renamed
let { foo: renamed } = { foo: "bar" };
assert.sameValue(renamed, "bar", "renamed binding");

// Object destructuring - missing property
let { present, missing } = { present: "here" };
assert.sameValue(present, "here", "present property");
assert.sameValue(missing, null, "missing is null");

// Object destructuring - nested
let { outer: { inner } } = { outer: { inner: "deep" } };
assert.sameValue(inner, "deep", "nested object destruct");

// Object destructuring - mixed with array
let { arr: [elem1, elem2] } = { arr: [10, 20] };
assert.sameValue(elem1, 10, "mixed elem1");
assert.sameValue(elem2, 20, "mixed elem2");

// Const destructuring
const [ca, cb] = [100, 200];
assert.sameValue(ca, 100, "const array destruct ca");
assert.sameValue(cb, 200, "const array destruct cb");

const { cx, cy } = { cx: 5, cy: 10 };
assert.sameValue(cx, 5, "const object destruct cx");
assert.sameValue(cy, 10, "const object destruct cy");

// Destructuring in function params
function processPoint([x, y]) {
    return x + y;
}
assert.sameValue(processPoint([3, 4]), 7, "function param array destruct");

function processPerson({ name, age }) {
    return name + " is " + age;
}
assert.sameValue(processPerson({ name: "Bob", age: 25 }), "Bob is 25", "function param object destruct");

printTestResults();
