// Hand-written JS reference for typed_array_of_structs.tish (type-erased).

let pts = [{ x: 1, y: 2 }, { x: 3, y: 4 }, { x: 5, y: 6 }];
console.log(pts[0].x);
console.log(pts[2].y);

let sum = 0;
for (let p of pts) {
  sum = sum + p.x + p.y;
}
console.log(sum);
