// Hand-written JS reference for typed_array_hof.tish (type-erased).

let xs = [3, 1, 4, 1, 5, 9, 2, 6];

let sum = xs.reduce((a, b) => a + b, 0);
console.log(sum);
let prod = xs.reduce((a, b) => a * b, 1);
console.log(prod);

let doubled = xs.map((x) => x * 2);
console.log(doubled.reduce((a, b) => a + b, 0));

let big = xs.filter((x) => x > 3);
console.log(big.length);
console.log(big.reduce((a, b) => a + b, 0));

let hasBig = xs.some((x) => x > 8);
console.log(hasBig);
let allPos = xs.every((x) => x > 0);
console.log(allPos);
let allBig = xs.every((x) => x > 3);
console.log(allBig);

let ys = [10, 20, 30];
console.log(ys.reduce((a, b) => a + b, 0));
