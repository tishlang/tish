// A braceless `if (cond) return` must NOT swallow the statement on the next line.
// Regression test for #96. (Translation of braceless_return.tish; the js-target
// oracle runs the tish-compiled output, this file gates that comparison.)

let log = [];
function rec(m) { log.push(m); }

function f(skip) {
  if (skip) return;
  rec("ran");
}
f(false);
console.log(JSON.stringify(log));          // ["ran"]

let log2 = [];
function rec2(m) { log2.push(m); }
function g(skip) {
  if (skip) return;
  rec2("ran2");
}
g(true);
console.log(JSON.stringify(log2));         // []

function h(x) {
  if (x > 0) return "pos";
  return "nonpos";
}
console.log(h(5), h(-1));                   // pos nonpos

function sum(a, b) {
  return a +
    b;
}
console.log(sum(3, 4));                     // 7
