// Test File I/O functions - Node.js equivalent of file_io.tish
const fs = require("fs");
const path = require("path");

let testDir = "/tmp/tish_test_" + Date.now();
let testFile = path.join(testDir, "test.txt");
let testContent = "Hello from Tish!";

// mkdir
fs.mkdirSync(testDir, { recursive: true });
let mkdirResult = fs.existsSync(testDir);
console.log("mkdir success:", mkdirResult === true);

// fileExists (should be false initially for file)
console.log("file doesn't exist yet:", fs.existsSync(testFile) === false);
console.log("dir exists:", fs.existsSync(testDir) === true);

// writeFile
fs.writeFileSync(testFile, testContent);
let writeResult = fs.existsSync(testFile);
console.log("writeFile success:", writeResult === true);

// fileExists after write
console.log("file exists after write:", fs.existsSync(testFile) === true);

// readFile
let content = fs.readFileSync(testFile, "utf8");
console.log("readFile content matches:", content === testContent);

// readDir
let files = fs.readdirSync(testDir);
console.log("readDir is array:", Array.isArray(files));
console.log("readDir contains file:", files.includes("test.txt"));

// Overwrite file
fs.writeFileSync(testFile, "Updated content");
let newContent = fs.readFileSync(testFile, "utf8");
console.log("file updated:", newContent === "Updated content");

// Cleanup
fs.unlinkSync(testFile);
fs.rmdirSync(testDir);

console.log("All file I/O tests completed");
