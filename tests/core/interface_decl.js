// Hand-written JS reference for interface_decl.tish (interface is type-only, erased).

let p = { x: 3, y: 4 };
console.log(p.x);
console.log(p.y);

function manhattan(a) {
  return a.x + a.y;
}
console.log(manhattan(p));

let pts = [{ x: 1, y: 1 }, { x: 2, y: 3 }];
let sum = 0;
for (let q of pts) {
  sum = sum + manhattan(q);
}
console.log(sum);
