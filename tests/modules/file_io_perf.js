// Performance test for File I/O operations

let testDir = "/tmp/tish_perf_" + Date.now();
let iterations = 1000;

// Setup
mkdir(testDir);

// Write performance
let startWrite = Date.now();
for (let i = 0; i < iterations; i = i + 1) {
    writeFile(testDir + "/file_" + i + ".txt", "content " + i);
}
let writeTime = Date.now() - startWrite;
console.log("writeFile x " + iterations + ": " + writeTime + "ms");

// Read performance
let startRead = Date.now();
for (let j = 0; j < iterations; j = j + 1) {
    readFile(testDir + "/file_" + j + ".txt");
}
let readTime = Date.now() - startRead;
console.log("readFile x " + iterations + ": " + readTime + "ms");

// fileExists performance
let startExists = Date.now();
for (let k = 0; k < iterations; k = k + 1) {
    fileExists(testDir + "/file_" + k + ".txt");
}
let existsTime = Date.now() - startExists;
console.log("fileExists x " + iterations + ": " + existsTime + "ms");

// readDir performance
let startDir = Date.now();
for (let l = 0; l < 100; l = l + 1) {
    readDir(testDir);
}
let dirTime = Date.now() - startDir;
console.log("readDir x 100: " + dirTime + "ms");

console.log("File I/O performance tests completed");
