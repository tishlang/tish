// Test262: continue statement

// Continue in for loop
let sum = 0;
for (let i = 0; i < 10; i++) {
    if (i % 2 === 0) {
        continue;
    }
    sum += i;
}
assert.sameValue(sum, 25, "continue skips even numbers (1+3+5+7+9)");

// Continue in while loop
let result = [];
let i = 0;
while (i < 10) {
    i += 1;
    if (i % 2 === 0) {
        continue;
    }
    result.push(i);
}
assert.sameValue(result.length, 5, "continue in while - odd numbers only");
assert.sameValue(result[0], 1, "continue in while - first odd");
assert.sameValue(result[4], 9, "continue in while - last odd");

// Continue only affects innermost loop
let outer = 0;
let inner = 0;
for (let i = 0; i < 3; i++) {
    outer += 1;
    for (let j = 0; j < 5; j++) {
        if (j === 2) {
            continue;
        }
        inner += 1;
    }
}
assert.sameValue(outer, 3, "continue inner - outer count");
assert.sameValue(inner, 12, "continue inner - inner count (3*4, skipping j=2)");

// Continue skips rest of iteration
let arr = [];
for (let i = 0; i < 5; i++) {
    arr.push("before" + i);
    if (i === 2) {
        continue;
    }
    arr.push("after" + i);
}
assert.sameValue(arr.length, 9, "continue skips rest - 5 befores + 4 afters");

// Filter pattern with continue
let numbers = [1, -2, 3, -4, 5, -6];
let positives = [];
for (let n of numbers) {
    if (n < 0) {
        continue;
    }
    positives.push(n);
}
assert.sameValue(positives.length, 3, "filter with continue - length");
assert.sameValue(positives[0], 1, "filter with continue[0]");
assert.sameValue(positives[1], 3, "filter with continue[1]");
assert.sameValue(positives[2], 5, "filter with continue[2]");

printTestResults();
