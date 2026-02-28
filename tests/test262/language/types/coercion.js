// Test262: Type coercion

// String to number coercion
assert.sameValue(+"42", 42, "unary + string to number");
assert.sameValue(+"3.14", 3.14, "unary + float string");
assert.sameValue(+"-5", -5, "unary + negative string");
assert.sameValue(+"0", 0, "unary + zero string");

// Number to string coercion
assert.sameValue("" + 42, "42", "number to string via concatenation");
assert.sameValue("" + 3.14, "3.14", "float to string");
assert.sameValue("" + -5, "-5", "negative to string");

// Boolean to number
assert.sameValue(+true, 1, "true to number");
assert.sameValue(+false, 0, "false to number");
assert.sameValue(true + 0, 1, "true + 0");
assert.sameValue(false + 0, 0, "false + 0");

// Boolean to string
assert.sameValue("" + true, "true", "true to string");
assert.sameValue("" + false, "false", "false to string");

// Null coercion
assert.sameValue(+null, 0, "null to number");
assert.sameValue("" + null, "null", "null to string");
assert.sameValue(null + 0, 0, "null + 0");

// Arithmetic coercion
assert.sameValue("10" - 5, 5, "string - number");
assert.sameValue("10" * 2, 20, "string * number");
assert.sameValue("20" / 4, 5, "string / number");
assert.sameValue("10" % 3, 1, "string % number");

// Addition is special (concatenates with strings)
assert.sameValue("10" + 5, "105", "string + number concatenates");
assert.sameValue(5 + "10", "510", "number + string concatenates");

// Comparison coercion
assert.sameValue("10" > 5, true, "string > number");
assert.sameValue("10" < 50, true, "string < number");
assert.sameValue("10" == 10, true, "string == number (loose)");
assert.sameValue("10" === 10, false, "string === number (strict)");

// Boolean in arithmetic
assert.sameValue(true + true, 2, "true + true");
assert.sameValue(true - false, 1, "true - false");
assert.sameValue(true * 5, 5, "true * 5");

// Falsy values
assert.sameValue(!!0, false, "0 is falsy");
assert.sameValue(!!"", false, "empty string is falsy");
assert.sameValue(!!null, false, "null is falsy");
assert.sameValue(!!false, false, "false is falsy");
assert.sameValue(!!NaN, false, "NaN is falsy");

// Truthy values
assert.sameValue(!!1, true, "1 is truthy");
assert.sameValue(!!"hello", true, "non-empty string is truthy");
assert.sameValue(!![], true, "empty array is truthy");
assert.sameValue(!!{}, true, "empty object is truthy");
assert.sameValue(!!-1, true, "-1 is truthy");

// Loose equality coercion
assert.sameValue(0 == false, true, "0 == false");
assert.sameValue(1 == true, true, "1 == true");
assert.sameValue("" == false, true, "'' == false");
assert.sameValue(null == null, true, "null == null");

printTestResults();
