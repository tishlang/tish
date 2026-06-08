// JIT probe — Node/V8 reference for tests/core/jit_probe.tish.
let start = 0;
let end = 0;
let arr = [];
for (let i = 0; i < 1000; i++) {
    arr.push(i);
}

start = Date.now();
let acc = 0;
for (let r = 0; r < 2000; r++) {
    acc = acc + arr.reduce((a, x) => a + x, 0);
}
end = Date.now();
console.log("01 reduce sum (JIT today):      " + (end - start) + "ms");

start = Date.now();
for (let r = 0; r < 2000; r++) {
    arr.map(x => x > 500 ? x * 2 : 0 - x);
}
end = Date.now();
console.log("02 map ternary (no JIT):        " + (end - start) + "ms");

start = Date.now();
for (let r = 0; r < 2000; r++) {
    arr.filter(x => x % 7 === 0);
}
end = Date.now();
console.log("03 filter mod (no JIT):         " + (end - start) + "ms");

start = Date.now();
for (let r = 0; r < 2000; r++) {
    arr.map(x => (x * 65599) & 65535);
}
end = Date.now();
console.log("04 map bitwise (no JIT):        " + (end - start) + "ms");

start = Date.now();
for (let r = 0; r < 2000; r++) {
    arr.map(x => Math.sqrt(x) + Math.sin(x));
}
end = Date.now();
console.log("05 map Math (no JIT):           " + (end - start) + "ms");

start = Date.now();
let s = 0;
for (let r = 0; r < 4000000; r++) {
    s = s + r * 2 - 1;
}
end = Date.now();
console.log("06 inline numeric loop (no JIT):" + (end - start) + "ms");

function fib(n) {
    if (n < 2) {
        return n;
    }
    return fib(n - 1) + fib(n - 2);
}
start = Date.now();
let f = fib(30);
end = Date.now();
console.log("07 recursion fib(30):           " + (end - start) + "ms (f=" + f + ")");

start = Date.now();
let str = "";
for (let r = 0; r < 50000; r++) {
    str = str + "x";
}
end = Date.now();
console.log("08 string concat loop:          " + (end - start) + "ms (len=" + str.length + ")");

start = Date.now();
let s2 = 0;
for (let r = 0; r < 4000; r++) {
    for (let i = 0; i < arr.length; i++) {
        s2 = s2 + arr[i];
    }
}
end = Date.now();
console.log("09 array index sum (no JIT):    " + (end - start) + "ms");

start = Date.now();
for (let r = 0; r < 2000; r++) {
    arr.find(x => x === 777);
}
end = Date.now();
console.log("10 find numeric (JIT today):    " + (end - start) + "ms");
