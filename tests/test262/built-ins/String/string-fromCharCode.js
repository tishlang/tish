// Test262: String.fromCharCode

// Basic ASCII
assert.sameValue(String.fromCharCode(65), "A", "fromCharCode A");
assert.sameValue(String.fromCharCode(66), "B", "fromCharCode B");
assert.sameValue(String.fromCharCode(90), "Z", "fromCharCode Z");

// Lowercase
assert.sameValue(String.fromCharCode(97), "a", "fromCharCode a");
assert.sameValue(String.fromCharCode(122), "z", "fromCharCode z");

// Numbers
assert.sameValue(String.fromCharCode(48), "0", "fromCharCode 0");
assert.sameValue(String.fromCharCode(57), "9", "fromCharCode 9");

// Special characters
assert.sameValue(String.fromCharCode(32), " ", "fromCharCode space");
assert.sameValue(String.fromCharCode(33), "!", "fromCharCode !");
assert.sameValue(String.fromCharCode(64), "@", "fromCharCode @");
assert.sameValue(String.fromCharCode(35), "#", "fromCharCode #");

// Control characters
assert.sameValue(String.fromCharCode(10), "\n", "fromCharCode newline");
assert.sameValue(String.fromCharCode(9), "\t", "fromCharCode tab");
assert.sameValue(String.fromCharCode(13), "\r", "fromCharCode carriage return");

// Multiple characters
assert.sameValue(String.fromCharCode(72, 105), "Hi", "fromCharCode multiple");
assert.sameValue(String.fromCharCode(65, 66, 67), "ABC", "fromCharCode ABC");

// Build string from codes
let codes = [72, 101, 108, 108, 111];
let str = "";
for (let code of codes) {
    str += String.fromCharCode(code);
}
assert.sameValue(str, "Hello", "build string from codes");

// Round-trip with charCodeAt
let original = "Test";
let rebuilt = "";
for (let i = 0; i < original.length; i++) {
    rebuilt += String.fromCharCode(original.charCodeAt(i));
}
assert.sameValue(rebuilt, original, "round-trip charCodeAt/fromCharCode");

// ASCII range
let alphabet = "";
for (let i = 65; i <= 90; i++) {
    alphabet += String.fromCharCode(i);
}
assert.sameValue(alphabet, "ABCDEFGHIJKLMNOPQRSTUVWXYZ", "uppercase alphabet");

let digits = "";
for (let i = 48; i <= 57; i++) {
    digits += String.fromCharCode(i);
}
assert.sameValue(digits, "0123456789", "digits");

// Verify specific char codes
assert.sameValue("A".charCodeAt(0), 65, "A is 65");
assert.sameValue("Z".charCodeAt(0), 90, "Z is 90");
assert.sameValue("a".charCodeAt(0), 97, "a is 97");
assert.sameValue("z".charCodeAt(0), 122, "z is 122");
assert.sameValue("0".charCodeAt(0), 48, "0 is 48");
assert.sameValue("9".charCodeAt(0), 57, "9 is 57");

printTestResults();
