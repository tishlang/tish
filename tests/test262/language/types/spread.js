// Test262: Spread operator

// Array spread - combine arrays
let arr1 = [1, 2, 3];
let arr2 = [4, 5, 6];
let combined = [...arr1, ...arr2];
assert.sameValue(combined.length, 6, "combined length");
assert.sameValue(combined[0], 1, "combined[0]");
assert.sameValue(combined[3], 4, "combined[3]");
assert.sameValue(combined[5], 6, "combined[5]");

// Array spread - insert in middle
let middle = [2, 3, 4];
let expanded = [1, ...middle, 5];
assert.sameValue(expanded.length, 5, "expanded length");
assert.sameValue(expanded[0], 1, "expanded[0]");
assert.sameValue(expanded[1], 2, "expanded[1]");
assert.sameValue(expanded[4], 5, "expanded[4]");

// Array spread - copy array
let original = [1, 2, 3];
let copy = [...original];
assert.sameValue(copy.length, 3, "copy length");
assert.sameValue(copy[0], 1, "copy[0]");
copy[0] = 100;
assert.sameValue(original[0], 1, "original unchanged after copy modification");

// Array spread - with elements
let prefix = [0, ...arr1];
assert.sameValue(prefix.length, 4, "prefix length");
assert.sameValue(prefix[0], 0, "prefix[0]");
assert.sameValue(prefix[1], 1, "prefix[1]");

let suffix = [...arr1, 100];
assert.sameValue(suffix.length, 4, "suffix length");
assert.sameValue(suffix[3], 100, "suffix[3]");

// Object spread - combine objects
let obj1 = { a: 1, b: 2 };
let obj2 = { c: 3, d: 4 };
let merged = { ...obj1, ...obj2 };
assert.sameValue(merged.a, 1, "merged.a");
assert.sameValue(merged.b, 2, "merged.b");
assert.sameValue(merged.c, 3, "merged.c");
assert.sameValue(merged.d, 4, "merged.d");

// Object spread - override properties
let base = { x: 1, y: 2 };
let override = { y: 20, z: 30 };
let result = { ...base, ...override };
assert.sameValue(result.x, 1, "result.x (from base)");
assert.sameValue(result.y, 20, "result.y (overridden)");
assert.sameValue(result.z, 30, "result.z (from override)");

// Object spread - copy
let origObj = { a: 1, b: 2 };
let copyObj = { ...origObj };
assert.sameValue(copyObj.a, 1, "copyObj.a");
copyObj.a = 100;
assert.sameValue(origObj.a, 1, "original object unchanged");

// Object spread - add properties
let extended = { ...obj1, e: 5 };
assert.sameValue(extended.a, 1, "extended.a");
assert.sameValue(extended.e, 5, "extended.e");

// Function call spread
function sum3(a, b, c) {
    return a + b + c;
}
let args = [1, 2, 3];
assert.sameValue(sum3(...args), 6, "function call spread");

// Mixed spread in function
function sum5(a, b, c, d, e) {
    return a + b + c + d + e;
}
let part1 = [1, 2];
let part2 = [4, 5];
assert.sameValue(sum5(...part1, 3, ...part2), 15, "mixed spread in call");

// Spread with string (iterates characters)
let chars = [..."abc"];
assert.sameValue(chars.length, 3, "string spread length");
assert.sameValue(chars[0], "a", "string spread[0]");
assert.sameValue(chars[1], "b", "string spread[1]");
assert.sameValue(chars[2], "c", "string spread[2]");

printTestResults();
