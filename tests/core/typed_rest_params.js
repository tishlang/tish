// Hand-written JS reference for typed_rest_params.tish (type-erased).

function sum(...args) {
  let total = 0;
  for (const n of args) {
    total = total + n;
  }
  return total;
}
console.log(sum(1, 2, 3, 4, 5));
console.log(sum(10, 20));

function maxOf(...xs) {
  let m = 0;
  for (const x of xs) {
    if (x > m) {
      m = x;
    }
  }
  return m;
}
console.log(maxOf(3, 9, 2, 7));

function scaledSum(k, ...rest) {
  let acc = 0;
  for (const r of rest) {
    acc = acc + r * k;
  }
  return acc;
}
console.log(scaledSum(2, 1, 2, 3));
