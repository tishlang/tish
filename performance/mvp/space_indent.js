// MVP test: space indentation (JS equivalent of space_indent.tish)
const x = 1;
if (x === 1) {
  console.log("space-indented");
  const y = 2;
  if (y === 2)
    console.log("nested with spaces");
}
console.log("done");
