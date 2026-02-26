// MVP test: optional braces - braced version (JS equivalent of optional_braces_braced.tish)
const n = 3;
if (n > 0) {
  console.log("positive");
  const x = 1;
  if (x === 1) {
    console.log("nested if, with braces");
  }
}
console.log("done");
