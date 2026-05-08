// Symbol: typeof, for/keyFor, ===, object index, in, JSON.stringify, Object.keys (JS equivalent of symbol.tish)
// Tish uses Symbol["for"] / Symbol["keyFor"] because `for` is a keyword in member position; JS uses .for / .keyFor.
console.log(typeof Symbol("z"));

const u1 = Symbol("u");
const u2 = Symbol("u");
console.log(u1 === u2);

const r1 = Symbol.for("sym.tish.test");
const r2 = Symbol.for("sym.tish.test");
console.log(r1 === r2);

const r3 = Symbol.for("sym.tish.other");
console.log(r1 === r3);

console.log(Symbol.keyFor(r1));
const kfU1 = Symbol.keyFor(u1);
console.log(kfU1 === undefined ? null : kfU1);

const sk = Symbol("sk");
const o = { a: 1 };
o[sk] = 42;
o["b"] = 2;
console.log(o["a"]);
console.log(o["b"]);
console.log(sk in o);
console.log("a" in o);
console.log("c" in o);
console.log(JSON.stringify(o));
const keys = Object.keys(o);
console.log(keys.length);
