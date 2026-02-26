// MVP test: for-of loop (iterate arrays and strings)
const arr = [10, 20, 30];
let sum = 0;
for (const x of arr)
  sum = sum + x;
console.log("array sum:", sum);

const s = "abc";
for (const c of s)
  console.log("char:", c);
