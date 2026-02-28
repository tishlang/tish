// Test262: Object literals

// Empty object
let empty = {};
assert.sameValue(typeof empty, "object", "empty object type");

// Simple properties
let person = { name: "Alice", age: 30 };
assert.sameValue(person.name, "Alice", "dot access name");
assert.sameValue(person.age, 30, "dot access age");
assert.sameValue(person["name"], "Alice", "bracket access name");
assert.sameValue(person["age"], 30, "bracket access age");

// Numeric values
let coords = { x: 10, y: 20, z: 30 };
assert.sameValue(coords.x, 10, "coords.x");
assert.sameValue(coords.y, 20, "coords.y");
assert.sameValue(coords.z, 30, "coords.z");

// String values
let texts = { greeting: "hello", farewell: "goodbye" };
assert.sameValue(texts.greeting, "hello", "string value");
assert.sameValue(texts.farewell, "goodbye", "string value 2");

// Boolean values
let flags = { enabled: true, visible: false };
assert.sameValue(flags.enabled, true, "boolean true");
assert.sameValue(flags.visible, false, "boolean false");

// Null value
let nullable = { value: null };
assert.sameValue(nullable.value, null, "null value");

// Nested objects
let nested = {
    outer: {
        inner: {
            value: "deep"
        }
    }
};
assert.sameValue(nested.outer.inner.value, "deep", "nested access");

// Array as value
let withArray = { items: [1, 2, 3] };
assert.sameValue(withArray.items.length, 3, "array property length");
assert.sameValue(withArray.items[0], 1, "array property element");

// Object as array element
let objInArray = { data: [{ x: 1 }, { x: 2 }] };
assert.sameValue(objInArray.data[0].x, 1, "object in array");
assert.sameValue(objInArray.data[1].x, 2, "object in array 2");

// Dynamic key access
let key = "name";
let obj = { name: "dynamic" };
assert.sameValue(obj[key], "dynamic", "dynamic key access");

// Missing property
let sparse = { a: 1 };
assert.sameValue(sparse.b, null, "missing property is null");
assert.sameValue(sparse["c"], null, "missing property bracket");

// Trailing comma
let trailing = { a: 1, b: 2, };
assert.sameValue(trailing.a, 1, "trailing comma works");
assert.sameValue(trailing.b, 2, "trailing comma works 2");

// Property with expression value
let a = 5;
let b = 10;
let computed = { sum: a + b, product: a * b };
assert.sameValue(computed.sum, 15, "computed sum");
assert.sameValue(computed.product, 50, "computed product");

// typeof
assert.sameValue(typeof {}, "object", "typeof empty object");
assert.sameValue(typeof { x: 1 }, "object", "typeof object");

printTestResults();
