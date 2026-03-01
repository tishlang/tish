// Test process object

// process.argv should be an array
console.log("process.argv is array:", Array.isArray(process.argv))
console.log("argv length >= 1:", process.argv.length >= 1)

// process.cwd should return a string
let cwd = process.cwd()
console.log("process.cwd() returns string:", typeof cwd === "string")
console.log("cwd is non-empty:", cwd.length > 0)

// process.env should be an object
console.log("process.env is object:", typeof process.env === "object")

// Check for common env vars (PATH usually exists)
let hasPath = "PATH" in process.env || "Path" in process.env
console.log("PATH env var exists:", hasPath)

// Note: process.exit() is not tested as it would terminate the test
console.log("process.exit exists:", typeof process.exit === "function")
