// Test262: Logical operators (&&, ||, !)

// Logical AND (&&)
assert.sameValue(true && true, true, "true && true");
assert.sameValue(true && false, false, "true && false");
assert.sameValue(false && true, false, "false && true");
assert.sameValue(false && false, false, "false && false");

// AND short-circuit
assert.sameValue(false && "never evaluated", false, "AND short-circuit");
assert.sameValue(0 && "never evaluated", 0, "falsy AND short-circuit");

// AND returns last truthy or first falsy
assert.sameValue(1 && 2, 2, "1 && 2 returns 2");
assert.sameValue("a" && "b", "b", "truthy AND returns right");
assert.sameValue(0 && 2, 0, "0 && 2 returns 0");
assert.sameValue(null && 2, null, "null && 2 returns null");

// Logical OR (||)
assert.sameValue(true || true, true, "true || true");
assert.sameValue(true || false, true, "true || false");
assert.sameValue(false || true, true, "false || true");
assert.sameValue(false || false, false, "false || false");

// OR short-circuit
assert.sameValue(true || "never evaluated", true, "OR short-circuit");
assert.sameValue(1 || "never evaluated", 1, "truthy OR short-circuit");

// OR returns first truthy or last falsy
assert.sameValue(1 || 2, 1, "1 || 2 returns 1");
assert.sameValue(0 || 2, 2, "0 || 2 returns 2");
assert.sameValue(null || "default", "default", "null || default");
assert.sameValue(false || 0, 0, "false || 0 returns 0");

// Logical NOT (!)
assert.sameValue(!true, false, "!true");
assert.sameValue(!false, true, "!false");
assert.sameValue(!0, true, "!0");
assert.sameValue(!1, false, "!1");
assert.sameValue(!"", true, "!empty string");
assert.sameValue(!"hello", false, "!non-empty string");
assert.sameValue(!null, true, "!null");

// Double negation
assert.sameValue(!!true, true, "!!true");
assert.sameValue(!!1, true, "!!1");
assert.sameValue(!!0, false, "!!0");
assert.sameValue(!!"hello", true, "!!string");

printTestResults();
