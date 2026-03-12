const scores = [85, 92, 78, 95, 88, 72];
const threshold = 80;

let passed = scores.filter(s => s >= threshold);
let count = passed.length;

console.log("Scores:", scores.join(", "));
console.log("Passed (>= " + threshold + "):", count, "-", passed.join(", "));

let sum = 0;
for (const s of scores) {
  sum = sum + s;
}
let avg = sum / scores.length;
console.log("Average:", avg);
