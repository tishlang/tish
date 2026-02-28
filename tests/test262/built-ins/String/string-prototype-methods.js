// Test262: String prototype methods

// indexOf
let str = "hello world";
assert.sameValue(str.indexOf("o"), 4, "indexOf first occurrence");
assert.sameValue(str.indexOf("world"), 6, "indexOf substring");
assert.sameValue(str.indexOf("x"), -1, "indexOf not found");
assert.sameValue(str.indexOf("o", 5), 7, "indexOf with fromIndex");

// includes
assert.sameValue(str.includes("world"), true, "includes found");
assert.sameValue(str.includes("xyz"), false, "includes not found");
assert.sameValue(str.includes("hello"), true, "includes at start");
assert.sameValue(str.includes("hello", 1), false, "includes with position");

// slice
assert.sameValue(str.slice(0, 5), "hello", "slice start");
assert.sameValue(str.slice(6), "world", "slice to end");
assert.sameValue(str.slice(-5), "world", "slice negative");
assert.sameValue(str.slice(0, -6), "hello", "slice negative end");

// substring
assert.sameValue(str.substring(0, 5), "hello", "substring");
assert.sameValue(str.substring(6), "world", "substring to end");
assert.sameValue(str.substring(6, 11), "world", "substring range");

// split
let parts = str.split(" ");
assert.sameValue(parts.length, 2, "split by space");
assert.sameValue(parts[0], "hello", "split[0]");
assert.sameValue(parts[1], "world", "split[1]");

parts = "a,b,c,d".split(",");
assert.sameValue(parts.length, 4, "split by comma");

parts = "hello".split("");
assert.sameValue(parts.length, 5, "split into chars");
assert.sameValue(parts[0], "h", "split char[0]");

// trim
assert.sameValue("  hello  ".trim(), "hello", "trim both");
assert.sameValue("  hello".trim(), "hello", "trim left");
assert.sameValue("hello  ".trim(), "hello", "trim right");
assert.sameValue("hello".trim(), "hello", "trim nothing");

// toUpperCase / toLowerCase
assert.sameValue("hello".toUpperCase(), "HELLO", "toUpperCase");
assert.sameValue("HELLO".toLowerCase(), "hello", "toLowerCase");
assert.sameValue("HeLLo".toUpperCase(), "HELLO", "toUpperCase mixed");
assert.sameValue("HeLLo".toLowerCase(), "hello", "toLowerCase mixed");

// startsWith / endsWith
str = "hello world";
assert.sameValue(str.startsWith("hello"), true, "startsWith found");
assert.sameValue(str.startsWith("world"), false, "startsWith not at start");
assert.sameValue(str.startsWith("world", 6), true, "startsWith with position");

assert.sameValue(str.endsWith("world"), true, "endsWith found");
assert.sameValue(str.endsWith("hello"), false, "endsWith not at end");

// replace
assert.sameValue("hello world".replace("world", "there"), "hello there", "replace");
assert.sameValue("hello hello".replace("hello", "hi"), "hi hello", "replace first only");

// replaceAll
assert.sameValue("hello hello".replaceAll("hello", "hi"), "hi hi", "replaceAll");
assert.sameValue("a-b-c".replaceAll("-", "_"), "a_b_c", "replaceAll dashes");

// charAt
str = "hello";
assert.sameValue(str.charAt(0), "h", "charAt 0");
assert.sameValue(str.charAt(4), "o", "charAt 4");
assert.sameValue(str.charAt(10), "", "charAt out of bounds");

// charCodeAt
assert.sameValue(str.charCodeAt(0), 104, "charCodeAt h");
assert.sameValue(str.charCodeAt(1), 101, "charCodeAt e");

// repeat
assert.sameValue("ab".repeat(3), "ababab", "repeat 3");
assert.sameValue("x".repeat(5), "xxxxx", "repeat 5");
assert.sameValue("hi".repeat(0), "", "repeat 0");

// padStart / padEnd
assert.sameValue("5".padStart(3, "0"), "005", "padStart with zeros");
assert.sameValue("42".padStart(5, " "), "   42", "padStart with spaces");
assert.sameValue("5".padEnd(3, "0"), "500", "padEnd with zeros");
assert.sameValue("hi".padEnd(5, "."), "hi...", "padEnd with dots");

printTestResults();
