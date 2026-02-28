// Test262: break statement

// Break in while loop
let count = 0;
while (true) {
    count += 1;
    if (count >= 5) {
        break;
    }
}
assert.sameValue(count, 5, "break in while loop");

// Break in for loop
let sum = 0;
for (let i = 0; i < 100; i++) {
    if (i >= 10) {
        break;
    }
    sum += i;
}
assert.sameValue(sum, 45, "break in for loop (0-9)");

// Break in do-while
count = 0;
do {
    count += 1;
    if (count >= 3) {
        break;
    }
} while (true);
assert.sameValue(count, 3, "break in do-while loop");

// Break only affects innermost loop
let outer = 0;
let inner = 0;
for (let i = 0; i < 5; i++) {
    outer += 1;
    for (let j = 0; j < 10; j++) {
        inner += 1;
        if (j >= 2) {
            break;
        }
    }
}
assert.sameValue(outer, 5, "break inner loop - outer count");
assert.sameValue(inner, 15, "break inner loop - inner count (5*3)");

// Break with condition
let result = [];
for (let i = 0; i < 10; i++) {
    if (i === 5) {
        break;
    }
    result.push(i);
}
assert.sameValue(result.length, 5, "break with condition - length");
assert.sameValue(result[4], 4, "break with condition - last element");

// Early break
let executed = false;
for (let i = 0; i < 10; i++) {
    break;
    executed = true;
}
assert.sameValue(executed, false, "code after break not executed");

printTestResults();
