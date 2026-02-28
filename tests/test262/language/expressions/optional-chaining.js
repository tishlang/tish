// Test262: Optional chaining operator (?.)

// Property access with optional chaining
let obj = { a: { b: { c: 1 } } };
assert.sameValue(obj?.a?.b?.c, 1, "nested property access");
assert.sameValue(obj?.x?.y?.z, null, "missing property returns null");

// Null/undefined base
let nullObj = null;
assert.sameValue(nullObj?.x, null, "null?.x is null");
assert.sameValue(nullObj?.x?.y, null, "null?.x?.y is null");

// Mixed access
let mixed = { a: null };
assert.sameValue(mixed?.a?.b, null, "property is null, chaining stops");

// Array access
let arr = [1, 2, 3];
assert.sameValue(arr?.[0], 1, "array optional access");
assert.sameValue(arr?.[10], null, "out of bounds optional access");

let nullArr = null;
assert.sameValue(nullArr?.[0], null, "null array optional access");

// Method calls (if supported)
let objWithMethod = {
    getValue: function() { return 42; }
};
assert.sameValue(objWithMethod?.getValue(), 42, "optional method call");

let objWithoutMethod = {};
assert.sameValue(objWithoutMethod?.getValue?.(), null, "missing method optional call");

// Deeply nested
let deep = {
    level1: {
        level2: {
            level3: {
                value: "deep"
            }
        }
    }
};
assert.sameValue(deep?.level1?.level2?.level3?.value, "deep", "deeply nested access");
assert.sameValue(deep?.level1?.missing?.level3?.value, null, "missing intermediate level");

printTestResults();
