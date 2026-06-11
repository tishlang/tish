// Hand-written JS reference for typed_param_loopbound.tish (type-erased).

function countUp(n) {
  let total = 0;
  for (let i = 0; i < n; i = i + 1) {
    total = total + i;
  }
  return total;
}
console.log(countUp(10));
console.log(countUp(100));
console.log(countUp(0));

function label(x) {
  return "v=" + x;
}
console.log(label(5));
console.log(label("hi"));
