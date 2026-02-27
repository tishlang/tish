// Regex performance tests
let iterations = 10000;

// Test 1: RegExp.test() performance
let testRe = RegExp("\\d+", "");
let testStr = "abc123def456ghi";
let start1 = Date.now();
for (let i = 0; i < iterations; i = i + 1) {
    testRe.test(testStr);
}
let end1 = Date.now();
console.log("RegExp.test() x " + iterations + ": " + (end1 - start1) + "ms");

// Test 2: RegExp.exec() performance
let execRe = RegExp("(\\d+)", "");
let start2 = Date.now();
for (let i = 0; i < iterations; i = i + 1) {
    execRe.exec(testStr);
}
let end2 = Date.now();
console.log("RegExp.exec() x " + iterations + ": " + (end2 - start2) + "ms");

// Test 3: String.match() global performance
let matchStr = "foo bar foo baz foo qux foo";
let matchRe = RegExp("foo", "g");
let start3 = Date.now();
for (let i = 0; i < iterations; i = i + 1) {
    matchStr.match(matchRe);
}
let end3 = Date.now();
console.log("String.match(global) x " + iterations + ": " + (end3 - start3) + "ms");

// Test 4: String.replace() global performance
let replaceStr = "hello world hello world";
let replaceRe = RegExp("hello", "g");
let start4 = Date.now();
for (let i = 0; i < iterations; i = i + 1) {
    replaceStr.replace(replaceRe, "hi");
}
let end4 = Date.now();
console.log("String.replace(global) x " + iterations + ": " + (end4 - start4) + "ms");

// Test 5: String.search() performance
let searchStr = "the quick brown fox jumps";
let searchRe = RegExp("fox", "");
let start5 = Date.now();
for (let i = 0; i < iterations; i = i + 1) {
    searchStr.search(searchRe);
}
let end5 = Date.now();
console.log("String.search() x " + iterations + ": " + (end5 - start5) + "ms");

// Test 6: String.split() with regex performance
let splitStr = "one1two2three3four";
let splitRe = RegExp("\\d", "");
let start6 = Date.now();
for (let i = 0; i < iterations; i = i + 1) {
    splitStr.split(splitRe);
}
let end6 = Date.now();
console.log("String.split(regex) x " + iterations + ": " + (end6 - start6) + "ms");

// Test 7: Case insensitive matching performance
let ciStr = "HELLO world HELLO";
let ciRe = RegExp("hello", "gi");
let start7 = Date.now();
for (let i = 0; i < iterations; i = i + 1) {
    ciStr.match(ciRe);
}
let end7 = Date.now();
console.log("Case insensitive match x " + iterations + ": " + (end7 - start7) + "ms");

console.log("=== Regex performance tests complete ===");
