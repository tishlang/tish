// Performance test: heavy object workload (JS equivalent)
// Stresses object creation, property access, and iteration

// Create many objects in a loop
const objects = [];
for (let i = 0; i < 100; i = i + 1) {
  // Note: We can't do objects.push(), so we create arrays with spread would be needed
  // For now, test object creation and access patterns
}

// Deep property access chains
const data = {
  users: {
    admin: {
      profile: {
        name: "Admin",
        level: 10
      }
    },
    guest: {
      profile: {
        name: "Guest",
        level: 1
      }
    }
  }
};

// Repeated deep access (simulates hot path)
let sum = 0;
for (let i = 0; i < 1000; i = i + 1) {
  sum = sum + data.users.admin.profile.level;
  sum = sum + data.users.guest.profile.level;
}
console.log("sum:", sum);

// Object property lookup with dynamic keys
const keys = ["a", "b", "c", "d", "e"];
const lookup = { a: 1, b: 2, c: 3, d: 4, e: 5 };
let total = 0;
for (const k of keys) {
  total = total + lookup[k];
}
console.log("total:", total);

// Nested object traversal
const tree = {
  val: 1,
  left: {
    val: 2,
    left: { val: 4, left: null, right: null },
    right: { val: 5, left: null, right: null }
  },
  right: {
    val: 3,
    left: { val: 6, left: null, right: null },
    right: { val: 7, left: null, right: null }
  }
};

// Simple tree value access
console.log("root:", tree.val);
console.log("left:", tree.left.val);
console.log("right:", tree.right.val);
console.log("left-left:", tree.left.left.val);
console.log("left-right:", tree.left.right.val);
console.log("right-left:", tree.right.left.val);
console.log("right-right:", tree.right.right.val);

// Many small object creations
function makeRecord(id, name, active) {
  return { id: id, name: name, active: active };
}

const records = [
  makeRecord(1, "Alice", true),
  makeRecord(2, "Bob", false),
  makeRecord(3, "Charlie", true),
  makeRecord(4, "Diana", true),
  makeRecord(5, "Eve", false)
];

let activeCount = 0;
for (const r of records) {
  if (r.active)
    activeCount = activeCount + 1;
}
console.log("active:", activeCount);

// Optional chaining performance
const maybeData = {
  level1: {
    level2: {
      level3: {
        value: 42
      }
    }
  }
};

let chainSum = 0;
for (let i = 0; i < 500; i = i + 1) {
  chainSum = chainSum + (maybeData?.level1?.level2?.level3?.value ?? 0);
}
console.log("chainSum:", chainSum);

// 'in' operator performance
const checkObj = { a: 1, b: 2, c: 3, d: 4, e: 5, f: 6, g: 7, h: 8, i: 9, j: 10 };
let inCount = 0;
const testKeys = ["a", "b", "z", "c", "d", "x", "e", "f", "y", "g"];
for (let i = 0; i < 100; i = i + 1) {
  for (const k of testKeys) {
    if (k in checkObj)
      inCount = inCount + 1;
  }
}
console.log("inCount:", inCount);
