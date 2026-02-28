// Test262: Equality operators (==, !=, ===, !==)

// Strict equality (===)
assert.sameValue(1 === 1, true, "1 === 1");
assert.sameValue(1 === 2, false, "1 === 2");
assert.sameValue("a" === "a", true, "string === string");
assert.sameValue(true === true, true, "true === true");
assert.sameValue(false === false, true, "false === false");
assert.sameValue(null === null, true, "null === null");

// Strict equality - type matters
assert.sameValue(1 === "1", false, "1 === '1' (different types)");
assert.sameValue(0 === false, false, "0 === false (different types)");
assert.sameValue("" === false, false, "'' === false (different types)");
assert.sameValue(null === false, false, "null === false");

// Strict inequality (!==)
assert.sameValue(1 !== 2, true, "1 !== 2");
assert.sameValue(1 !== 1, false, "1 !== 1");
assert.sameValue(1 !== "1", true, "1 !== '1' (different types)");

// Loose equality (==)
assert.sameValue(1 == 1, true, "1 == 1");
assert.sameValue(1 == "1", true, "1 == '1' (coercion)");
assert.sameValue(0 == false, true, "0 == false");
assert.sameValue("" == false, true, "'' == false");
assert.sameValue(null == null, true, "null == null");

// Loose inequality (!=)
assert.sameValue(1 != 2, true, "1 != 2");
assert.sameValue(1 != "1", false, "1 != '1' (coercion)");

// Object/array equality (reference)
let arr1 = [1, 2, 3];
let arr2 = [1, 2, 3];
let arr3 = arr1;
assert.sameValue(arr1 === arr2, false, "different arrays with same content");
assert.sameValue(arr1 === arr3, true, "same array reference");

let obj1 = { x: 1 };
let obj2 = { x: 1 };
let obj3 = obj1;
assert.sameValue(obj1 === obj2, false, "different objects with same content");
assert.sameValue(obj1 === obj3, true, "same object reference");

printTestResults();
