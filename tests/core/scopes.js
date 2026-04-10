// Reference JS for the same prints as scopes.tish (Tish is simpler: no var hoisting, no let/const TDZ).
let x = 1;
console.log("outer x:", x);
{
  let x = 2;
  console.log("inner x:", x);
}
console.log("outer x after block:", x);

let y = 10;
{
  let y = 20;
  console.log("inner y:", y);
}
console.log("outer y:", y);

let a = 100;
{
  let a = 200;
  console.log("block1 a:", a);
  {
    let a = 300;
    console.log("block2 a:", a);
  }
  console.log("block1 a after inner:", a);
}
console.log("outer a:", a);

let z = 1;
function g() {
  let z = 2;
  console.log("fn z:", z);
}
g();
console.log("script z:", z);

let b = 1;
if (true) {
  let b = 2;
  console.log("if block b:", b);
}
console.log("outer b after if:", b);
