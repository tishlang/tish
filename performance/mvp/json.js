// MVP perf: JSON.parse and JSON.stringify
const obj = { x: 1, y: "hi", z: true };
const s = JSON.stringify(obj);
console.log(s);

const parsed = JSON.parse(s);
console.log(parsed.x);
console.log(parsed.y);
console.log(parsed.z);

const arr = [1, 2, "a"];
console.log(JSON.stringify(arr));

const arr2 = JSON.parse('[1,2,"a"]');
console.log(arr2[0]);
console.log(arr2[1]);
console.log(arr2[2]);
