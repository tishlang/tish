// Hand-written JS reference for typed_array_literal_infer.tish (type-erased).

let nums = [10, 20, 30, 40];
let total = 0;
for (let n of nums) {
  total = total + n;
}
console.log(total);
console.log(nums.length);

let names = ["a", "b", "c"];
let joined = "";
for (let s of names) {
  joined = joined + s;
}
console.log(joined);
