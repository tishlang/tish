// MVP test: arrays and indexing (JS equivalent of arrays.tish)
const arr = [1, 2, 3];
console.log(arr[0]);
console.log(arr[1]);
console.log(arr[2]);
let sum = 0;
for (let i = 0; i < 3; i = i + 1)
  sum = sum + arr[i];
console.log("sum:", sum);
const nested = [[1, 2], [3, 4]];
console.log(nested[0][1]);
console.log(nested[1][0]);
