// MVP test: optional braces - indentation logic (JS equivalent of optional_braces.tish)
const n = 3;
if (n > 0) {
  console.log("positive");
  const x = 1;
  if (x === 1)
    console.log("nested if, no braces");
}
console.log("done");
