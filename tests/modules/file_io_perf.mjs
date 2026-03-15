// Performance test for File I/O operations
// Compatible with Node.js, Bun, and Deno (ESM with node: specifiers)

import * as fs from "node:fs";
import * as path from "node:path";

let testDir = "/tmp/tish_perf_" + Date.now();
let iterations = 1000;

// Setup
fs.mkdirSync(testDir, { recursive: true });

// Write performance (use writeFileSync - writeFile is async and needs callback)
let startWrite = Date.now();
for (let i = 0; i < iterations; i = i + 1) {
  fs.writeFileSync(
    path.join(testDir, "file_" + i + ".txt"),
    "content " + i
  );
}
let writeTime = Date.now() - startWrite;
console.log("writeFile x " + iterations + ": " + writeTime + "ms");

// Read performance
let startRead = Date.now();
for (let j = 0; j < iterations; j = j + 1) {
  fs.readFileSync(path.join(testDir, "file_" + j + ".txt"));
}
let readTime = Date.now() - startRead;
console.log("readFile x " + iterations + ": " + readTime + "ms");

// fileExists performance (Node uses fs.existsSync, not fs.fileExists)
let startExists = Date.now();
for (let k = 0; k < iterations; k = k + 1) {
  fs.existsSync(path.join(testDir, "file_" + k + ".txt"));
}
let existsTime = Date.now() - startExists;
console.log("fileExists x " + iterations + ": " + existsTime + "ms");

// readDir performance (Node uses fs.readdirSync, not fs.readDirSync)
let startDir = Date.now();
for (let l = 0; l < 100; l = l + 1) {
  fs.readdirSync(testDir);
}
let dirTime = Date.now() - startDir;
console.log("readDir x 100: " + dirTime + "ms");

// Cleanup
for (let i = 0; i < iterations; i = i + 1) {
  fs.unlinkSync(path.join(testDir, "file_" + i + ".txt"));
}
fs.rmdirSync(testDir);

console.log("File I/O performance tests completed");
