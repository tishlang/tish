// MVP test: nested loops - multiplication table (JS equivalent of nested_loops.tish)
let i = 0;
let j = 0;
for (let row = 1; row <= 3; row = row + 1) {
  for (let col = 1; col <= 3; col = col + 1) {
    console.log(row, "x", col, "=", row * col);
  }
}
console.log("i after loops:", i);
console.log("j after loops:", j);
