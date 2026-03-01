// Comprehensive test: plain objects and property access (JS equivalent)
// Tests: literals, dot access, bracket access, nested objects, objects with functions
// NOTE: JS returns undefined where Tish returns null (semantic difference)

// Basic object creation
const pt = { x: 10, y: 20 };
console.log(pt.x);
console.log(pt.y);

// Bracket access with string literal
console.log(pt["x"]);

// Dynamic property access
const propName = "y";
console.log(pt[propName]);

// Multiple properties
const rec = { a: 1, b: 2, c: 3 };
console.log(rec.a, rec.b, rec.c);

// Nested objects
const nested = {
  outer: {
    inner: {
      value: 42
    }
  }
};
console.log(nested.outer.inner.value);
console.log(nested["outer"]["inner"]["value"]);

// Mixed dot and bracket access
console.log(nested.outer["inner"].value);

// Objects with different value types
const mixed = {
  num: 123,
  str: "hello",
  bool: true,
  nil: null,
  arr: [1, 2, 3],
  obj: { nested: true }
};
console.log(mixed.num);
console.log(mixed.str);
console.log(mixed.bool);
console.log(mixed.nil);
console.log(mixed.arr[1]);
console.log(mixed.obj.nested);

// Dynamic property access with bracket notation
const dynamicKey = "x";
console.log(pt[dynamicKey]);

// Objects as function parameters
const getX = (obj) => obj.x;
const getY = (obj) => obj.y;
console.log(getX(pt));
console.log(getY(pt));

// Functions returning objects
function makePoint(x, y) {
  return { x: x, y: y };
}
const p = makePoint(100, 200);
console.log(p.x, p.y);

// Object in conditional
const config = { enabled: true, count: 5 };
if (config.enabled)
  console.log("enabled with count:", config.count);

// Object in loop iteration (accessing properties)
const items = [
  { name: "a", val: 1 },
  { name: "b", val: 2 },
  { name: "c", val: 3 }
];
for (const item of items)
  console.log(item.name, item.val);

// typeof object
console.log(typeof {});
console.log(typeof { x: 1 });
console.log(typeof nested);

// Object equality (reference comparison)
const obj1 = { x: 1 };
const obj2 = { x: 1 };
const obj3 = obj1;
console.log(obj1 === obj2);
console.log(obj1 === obj3);

// 'in' operator
const testIn = { a: 1, b: 2 };
console.log("a" in testIn);
console.log("c" in testIn);

// Optional chaining with objects
const maybeNull = null;
console.log(maybeNull?.x);
const hasX = { x: 99 };
console.log(hasX?.x);

// Deep optional chaining
const deep = { a: { b: { c: 1 } } };
const shallow = { a: null };
console.log(deep?.a?.b?.c);
console.log(shallow?.a?.b?.c);

// Nullish coalescing with object properties
const withNull = { val: null };
const withValue = { val: 42 };
console.log(withNull.val ?? "default");
console.log(withValue.val ?? "default");
