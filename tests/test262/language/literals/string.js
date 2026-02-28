// Test262: String literals

// Basic strings
assert.sameValue("hello", "hello", "double quoted string");
assert.sameValue('hello', 'hello', "single quoted string");
assert.sameValue("", "", "empty string double quotes");
assert.sameValue('', '', "empty string single quotes");

// String equality
assert.sameValue("hello" === "hello", true, "same strings are equal");
assert.sameValue("hello" === "world", false, "different strings not equal");
assert.sameValue("Hello" === "hello", false, "case sensitive");

// String with spaces
assert.sameValue("hello world", "hello world", "string with space");
assert.sameValue("  leading", "  leading", "leading spaces");
assert.sameValue("trailing  ", "trailing  ", "trailing spaces");

// Escape sequences
assert.sameValue("line1\nline2".length, 11, "newline escape length");
assert.sameValue("col1\tcol2".length, 9, "tab escape length");
assert.sameValue("quote: \"hi\"", "quote: \"hi\"", "escaped double quote");
assert.sameValue('quote: \'hi\'', "quote: 'hi'", "escaped single quote");
assert.sameValue("back\\slash", "back\\slash", "escaped backslash");

// String length
assert.sameValue("".length, 0, "empty string length");
assert.sameValue("a".length, 1, "single char length");
assert.sameValue("hello".length, 5, "hello length");
assert.sameValue("hello world".length, 11, "hello world length");

// String indexing
let str = "hello";
assert.sameValue(str[0], "h", "index 0");
assert.sameValue(str[1], "e", "index 1");
assert.sameValue(str[4], "o", "index 4");

// String concatenation
assert.sameValue("hello" + " " + "world", "hello world", "concatenation");
assert.sameValue("a" + "b" + "c", "abc", "multiple concatenation");

// String + number coercion
assert.sameValue("value: " + 42, "value: 42", "string + number");
assert.sameValue(42 + " is the answer", "42 is the answer", "number + string");

// typeof
assert.sameValue(typeof "hello", "string", "typeof string");
assert.sameValue(typeof "", "string", "typeof empty string");

// Unicode (basic)
assert.sameValue("abc".length, 3, "ascii length");

printTestResults();
