// MVP test: break and continue in nested loops (JS equivalent of break_continue.tish)
let found = 0;
for (let i = 0; i < 3; i = i + 1) {
  for (let j = 0; j < 3; j = j + 1) {
    if (j === 1)
      continue;
    found = found + 1;
    if (i === 2 && j === 2)
      break;
  }
}
console.log("found:", found);
let count = 0;
for (let i = 0; i < 5; i = i + 1) {
  if (i === 3)
    break;
  count = count + 1;
}
console.log("count:", count);
