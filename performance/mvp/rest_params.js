// MVP test: rest parameters
function sum(...args) {
  let total = 0;
  for (const x of args)
    total = total + x;
  return total;
}
console.log(sum(1, 2, 3));
console.log(sum(10, 20));

function greet(first, ...rest) {
  return first + ":" + rest.length;
}
console.log(greet("a", "b", "c"));
