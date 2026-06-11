// Hand-written JS reference for typed_array_forof.tish (type-erased).

let xs = [3, 1, 4, 1, 5, 9, 2, 6];
let total = 0;
for (let x of xs) {
  total = total + x;
}
console.log(total);

let prod = 1;
for (let x of xs) {
  prod = prod * x;
}
console.log(prod);

let words = ["a", "b", "c"];
let joined = "";
for (let w of words) {
  joined = joined + w;
}
console.log(joined);

let ys = [10, 20, 30];
let s = 0;
for (let y of ys) {
  s = s + y;
}
console.log(s);
