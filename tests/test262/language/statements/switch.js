// Test262: switch/case/default statements

// Basic switch
let x = 2;
let result = "";
switch (x) {
    case 1:
        result = "one";
        break;
    case 2:
        result = "two";
        break;
    case 3:
        result = "three";
        break;
}
assert.sameValue(result, "two", "basic switch case 2");

// Switch with default
x = 10;
switch (x) {
    case 1:
        result = "one";
        break;
    case 2:
        result = "two";
        break;
    default:
        result = "default";
}
assert.sameValue(result, "default", "switch default case");

// Switch with string
let str = "hello";
switch (str) {
    case "hi":
        result = "informal";
        break;
    case "hello":
        result = "greeting";
        break;
    case "goodbye":
        result = "farewell";
        break;
}
assert.sameValue(result, "greeting", "switch with string");

// Fall-through (no break)
x = 1;
result = "";
switch (x) {
    case 1:
        result += "one";
    case 2:
        result += "two";
    case 3:
        result += "three";
        break;
    default:
        result += "default";
}
assert.sameValue(result, "onetwothree", "fall-through without break");

// Default in middle
x = 5;
result = "";
switch (x) {
    case 1:
        result = "one";
        break;
    default:
        result = "default";
        break;
    case 2:
        result = "two";
        break;
}
assert.sameValue(result, "default", "default in middle");

// Empty case
x = 1;
result = "";
switch (x) {
    case 1:
    case 2:
    case 3:
        result = "1, 2, or 3";
        break;
    default:
        result = "other";
}
assert.sameValue(result, "1, 2, or 3", "grouped cases (1)");

x = 3;
switch (x) {
    case 1:
    case 2:
    case 3:
        result = "1, 2, or 3";
        break;
    default:
        result = "other";
}
assert.sameValue(result, "1, 2, or 3", "grouped cases (3)");

// Switch with expression
x = 5;
switch (x * 2) {
    case 10:
        result = "ten";
        break;
    default:
        result = "other";
}
assert.sameValue(result, "ten", "switch with expression");

printTestResults();
