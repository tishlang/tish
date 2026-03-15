// HTTP fetch tests - Node.js/Deno equivalent of http_fetch.tish
// Node 18+ and Deno have built-in fetch

(async function main() {
  // Test 1: Simple GET request
  let response = await fetch("https://httpbin.org/get");
  console.log("GET status:", response.status);
  console.log("GET ok:", response.ok);

  // Test 2: POST request with JSON body
  let postOptions = { method: "POST", body: '{"name": "tish"}' };
  let postResponse = await fetch("https://httpbin.org/post", postOptions);
  console.log("POST status:", postResponse.status);
  console.log("POST ok:", postResponse.ok);

  // Test 3: Parse JSON response
  let jsonResponse = await fetch("https://httpbin.org/json");
  if (jsonResponse.ok) {
    let data = await jsonResponse.json();
    console.log("JSON response parsed successfully");
  }

  // Test 4: fetchAll equivalent - parallel requests with Promise.all
  let req1 = fetch("https://httpbin.org/get");
  let req2 = fetch("https://httpbin.org/status/200");
  let req3 = fetch("https://httpbin.org/headers");
  let responses = await Promise.all([req1, req2, req3]);
  console.log("Parallel fetch count:", responses.length);
  console.log(
    "All responses ok:",
    responses[0].ok && responses[1].ok && responses[2].ok
  );

  console.log("HTTP tests completed");
})();
