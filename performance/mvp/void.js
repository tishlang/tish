// MVP test: void operator (evaluates operand, returns undefined)
const x = void (1 + 2);
console.log(typeof x);
console.log(x === undefined);
const dummy = void console.log("side effect");
