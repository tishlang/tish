// Test Object.keys, Object.values, Object.entries

let obj = { name: "Alice", age: 30, city: "NYC" };

// Object.keys
let keys = Object.keys(obj);
console.log("keys count:", keys.length);

// Object.values
let vals = Object.values(obj);
console.log("values count:", vals.length);

// Object.entries
let entries = Object.entries(obj);
console.log("entries count:", entries.length);

// Iterate with forEach over entries (sort for deterministic order)
let sortedEntries = Object.entries(obj).sort((a, b) => a[0].localeCompare(b[0]));
function printEntry(entry) {
    console.log("  key:", entry[0], "val:", entry[1]);
}
console.log("entries:");
sortedEntries.forEach(printEntry);

// Use with map
function getKey(entry) { return entry[0]; }
let keyNames = Object.entries(obj).map(getKey).sort();
console.log("mapped keys:", keyNames.join(","));

// Empty object
let empty = {};
console.log("empty keys:", Object.keys(empty).length);
console.log("empty values:", Object.values(empty).length);
console.log("empty entries:", Object.entries(empty).length);

// Nested object
let nested = {
    a: 1,
    b: { x: 10, y: 20 },
    c: [1, 2, 3]
};
console.log("nested keys:", Object.keys(nested).length);
