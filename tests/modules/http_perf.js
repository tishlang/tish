// HTTP performance test - fetch operations
// Node.js/Bun compatible (uses async fetch)

(async function () {
  // Test 1: Sequential single fetches
  console.log("Testing sequential fetches...");
  let sequentialCount = 5;
  let i = 0;
  while (i < sequentialCount) {
    let response = await fetch("https://httpbin.org/get");
    if (!response.ok) {
      console.log("Sequential fetch failed:", response.status);
    }
    i = i + 1;
  }
  console.log("Sequential fetches completed:", sequentialCount);

  // Test 2: Parallel fetches with Promise.all (equivalent to fetchAll)
  console.log("Testing parallel fetches...");
  let parallelRequests = [
    "https://httpbin.org/get",
    "https://httpbin.org/get",
    "https://httpbin.org/get",
    "https://httpbin.org/get",
    "https://httpbin.org/get",
  ];
  let parallelResponses = await Promise.all(
    parallelRequests.map((url) => fetch(url))
  );
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
    fetch("https://httpbin.org/get"),
    fetch("https://httpbin.org/post", {
      method: "POST",
      body: '{"test": 1}',
    }),
    fetch("https://httpbin.org/put", {
      method: "PUT",
      body: '{"test": 2}',
    }),
    fetch("https://httpbin.org/delete", { method: "DELETE" }),
  ];
  let mixedResponses = await Promise.all(mixedRequests);
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
  let largeResponse = await fetch("https://httpbin.org/bytes/10000");
  if (largeResponse.ok) {
    console.log("Large response status:", largeResponse.status);
  }

  // Test 5: JSON parsing from response
  console.log("Testing JSON response parsing...");
  let jsonResponse = await fetch("https://httpbin.org/json");
  if (jsonResponse.ok) {
    let data = await jsonResponse.json();
    console.log("JSON parsed successfully");
  }

  console.log("HTTP performance tests completed");
})();
