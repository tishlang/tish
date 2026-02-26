// MVP test: plain objects and dot/index access (JS equivalent of objects.tish)
const pt = { x: 10, y: 20 };
console.log(pt.x);
console.log(pt.y);
console.log(pt["x"]);
const name = "y";
console.log(pt[name]);
const rec = { a: 1, b: 2, c: 3 };
console.log(rec.a, rec.b, rec.c);
