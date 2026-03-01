// Test higher-order array methods

let nums = [1, 2, 3, 4, 5];

// Define callback functions
function double(x) { return x * 2; }
function isEven(x) { return x % 2 === 0; }
function add(acc, x) { return acc + x; }
function gtThree(x) { return x > 3; }
function gtZero(x) { return x > 0; }
function isThree(x) { return x === 3; }
function ltThree(x) { return x < 3; }

// map
let doubled = nums.map(double);
console.log("map:", doubled.join(","));

// filter
let evens = nums.filter(isEven);
console.log("filter:", evens.join(","));

// reduce with initial value
let sum = nums.reduce(add, 0);
console.log("reduce:", sum);

// reduce without initial value
let product = nums.reduce(function(acc, x) { return acc * x; });
console.log("reduce (no init):", product);

// find
let found = nums.find(gtThree);
console.log("find:", found);

// find returns undefined when not found (we'll use null in output)
let notFound = nums.find(function(x) { return x > 100; });
console.log("find (not found):", notFound === undefined ? "null" : notFound);

// findIndex
let foundIdx = nums.findIndex(gtThree);
console.log("findIndex:", foundIdx);

// findIndex returns -1 when not found
let notFoundIdx = nums.findIndex(function(x) { return x > 100; });
console.log("findIndex (not found):", notFoundIdx);

// some
console.log("some (> 3):", nums.some(gtThree));
console.log("some (> 100):", nums.some(function(x) { return x > 100; }));

// every
console.log("every (> 0):", nums.every(gtZero));
console.log("every (> 3):", nums.every(gtThree));

// forEach
let total = 0;
function addToTotal(x) { total = total + x; }
nums.forEach(addToTotal);
console.log("forEach total:", total);

// flat
let nested = [1, [2, 3], [4, [5, 6]]];
let flat1 = nested.flat();
let flat1Str = flat1.map(x => Array.isArray(x) ? "[" + x.join(", ") + "]" : x).join(",");
console.log("flat(1):", flat1Str);
console.log("flat(2):", nested.flat(2).join(","));

// Chaining
let result = [1, 2, 3, 4, 5, 6]
    .filter(isEven)
    .map(double);
console.log("chained:", result.join(","));
