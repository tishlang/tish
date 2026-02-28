// Test262: JSON.parse and JSON.stringify

// JSON.parse - numbers
assert.sameValue(JSON.parse("42"), 42, "parse integer");
assert.sameValue(JSON.parse("3.14"), 3.14, "parse float");
assert.sameValue(JSON.parse("-5"), -5, "parse negative");
assert.sameValue(JSON.parse("0"), 0, "parse zero");

// JSON.parse - strings
assert.sameValue(JSON.parse('"hello"'), "hello", "parse string");
assert.sameValue(JSON.parse('""'), "", "parse empty string");
assert.sameValue(JSON.parse('"hello world"'), "hello world", "parse string with space");

// JSON.parse - booleans
assert.sameValue(JSON.parse("true"), true, "parse true");
assert.sameValue(JSON.parse("false"), false, "parse false");

// JSON.parse - null
assert.sameValue(JSON.parse("null"), null, "parse null");

// JSON.parse - arrays
let arr = JSON.parse("[1, 2, 3]");
assert.sameValue(Array.isArray(arr), true, "parse array is array");
assert.sameValue(arr.length, 3, "parse array length");
assert.sameValue(arr[0], 1, "parse array[0]");
assert.sameValue(arr[2], 3, "parse array[2]");

// JSON.parse - empty array
arr = JSON.parse("[]");
assert.sameValue(arr.length, 0, "parse empty array");

// JSON.parse - nested arrays
arr = JSON.parse("[[1, 2], [3, 4]]");
assert.sameValue(arr[0][0], 1, "parse nested array");
assert.sameValue(arr[1][1], 4, "parse nested array[1][1]");

// JSON.parse - objects
let obj = JSON.parse('{"name": "Alice", "age": 30}');
assert.sameValue(obj.name, "Alice", "parse object.name");
assert.sameValue(obj.age, 30, "parse object.age");

// JSON.parse - empty object
obj = JSON.parse("{}");
assert.sameValue(Object.keys(obj).length, 0, "parse empty object");

// JSON.parse - nested objects
obj = JSON.parse('{"outer": {"inner": "value"}}');
assert.sameValue(obj.outer.inner, "value", "parse nested object");

// JSON.parse - mixed
obj = JSON.parse('{"arr": [1, 2, 3], "str": "hello", "num": 42}');
assert.sameValue(obj.arr.length, 3, "parse mixed - array");
assert.sameValue(obj.str, "hello", "parse mixed - string");
assert.sameValue(obj.num, 42, "parse mixed - number");

// JSON.stringify - numbers
assert.sameValue(JSON.stringify(42), "42", "stringify integer");
assert.sameValue(JSON.stringify(3.14), "3.14", "stringify float");
assert.sameValue(JSON.stringify(-5), "-5", "stringify negative");

// JSON.stringify - strings
assert.sameValue(JSON.stringify("hello"), '"hello"', "stringify string");
assert.sameValue(JSON.stringify(""), '""', "stringify empty string");

// JSON.stringify - booleans
assert.sameValue(JSON.stringify(true), "true", "stringify true");
assert.sameValue(JSON.stringify(false), "false", "stringify false");

// JSON.stringify - null
assert.sameValue(JSON.stringify(null), "null", "stringify null");

// JSON.stringify - arrays
assert.sameValue(JSON.stringify([1, 2, 3]), "[1,2,3]", "stringify array");
assert.sameValue(JSON.stringify([]), "[]", "stringify empty array");

// JSON.stringify - objects
let str = JSON.stringify({ a: 1, b: 2 });
// Order may vary, so parse back and check
let parsed = JSON.parse(str);
assert.sameValue(parsed.a, 1, "stringify->parse object.a");
assert.sameValue(parsed.b, 2, "stringify->parse object.b");

// JSON.stringify - nested
let nested = { outer: { inner: "deep" }, arr: [1, 2] };
str = JSON.stringify(nested);
parsed = JSON.parse(str);
assert.sameValue(parsed.outer.inner, "deep", "stringify nested");
assert.sameValue(parsed.arr.length, 2, "stringify nested array");

// Round-trip
let original = { name: "Bob", scores: [100, 95, 87], active: true };
let roundTrip = JSON.parse(JSON.stringify(original));
assert.sameValue(roundTrip.name, "Bob", "round-trip name");
assert.sameValue(roundTrip.scores.length, 3, "round-trip scores");
assert.sameValue(roundTrip.active, true, "round-trip active");

printTestResults();
