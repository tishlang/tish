// HTTP performance test - fetch and fetchAll operations
// Note: Requires network access and the http feature enabled

// Test 1: Sequential single fetches
console.log("Testing sequential fetches...");
let sequentialCount = 5;
let i = 0;
while (i < sequentialCount) {
    let response = fetch("https://httpbin.org/get");
    if (!response.ok) {
        console.log("Sequential fetch failed:", response.status);
    }
    i = i + 1;
}
console.log("Sequential fetches completed:", sequentialCount);

// Test 2: Parallel fetches with fetchAll
console.log("Testing parallel fetches...");
let parallelRequests = [
    { url: "https://httpbin.org/get" },
    { url: "https://httpbin.org/get" },
    { url: "https://httpbin.org/get" },
    { url: "https://httpbin.org/get" },
    { url: "https://httpbin.org/get" }
];
let parallelResponses = fetchAll(parallelRequests);
let parallelOk = 0;
i = 0;
while (i < parallelResponses.length) {
    if (parallelResponses[i].ok) {
        parallelOk = parallelOk + 1;
    }
    i = i + 1;
}
console.log("Parallel fetches completed:", parallelOk, "/", parallelResponses.length);

// Test 3: Mixed methods
console.log("Testing mixed HTTP methods...");
let mixedRequests = [
    { url: "https://httpbin.org/get" },
    { url: "https://httpbin.org/post", method: "POST", body: "{\"test\": 1}" },
    { url: "https://httpbin.org/put", method: "PUT", body: "{\"test\": 2}" },
    { url: "https://httpbin.org/delete", method: "DELETE" }
];
let mixedResponses = fetchAll(mixedRequests);
let mixedOk = 0;
i = 0;
while (i < mixedResponses.length) {
    if (mixedResponses[i].ok) {
        mixedOk = mixedOk + 1;
    }
    i = i + 1;
}
console.log("Mixed method fetches:", mixedOk, "/", mixedResponses.length);

// Test 4: Large response handling
console.log("Testing larger response...");
let largeResponse = fetch("https://httpbin.org/bytes/10000");
if (largeResponse.ok) {
    console.log("Large response status:", largeResponse.status);
}

// Test 5: JSON parsing from response
console.log("Testing JSON response parsing...");
let jsonResponse = fetch("https://httpbin.org/json");
if (jsonResponse.ok) {
    let data = JSON.parse(jsonResponse.body);
    console.log("JSON parsed successfully");
}

console.log("HTTP performance tests completed");
