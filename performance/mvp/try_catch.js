// MVP test: try/catch and throw (JS equivalent of try_catch.tish)
try {
  console.log("in try");
  throw "oops";
  console.log("never");
} catch (e) {
  console.log("caught:", e);
}
console.log("done");
try {
  throw 42;
} catch (x) {
  console.log("got", x);
}
