// Combined validation: async/await + Promise + setTimeout + multiple HTTP requests
// Node.js equivalent of async_promise_settimeout.tish

(async () => {
  console.log("ASYNC_VALIDATION_START");

  // 1. setTimeout (non-blocking) - callbacks run after main script
  setTimeout(() => {
    console.log("TIMER_1_FIRED");
  }, 0);
  setTimeout(() => {
    console.log("TIMER_2_FIRED");
  }, 20);

  // 2. Promise + await
  let p = Promise.resolve("ok");
  let v = await p;
  console.log("PROMISE_AWAIT:", v);

  // 3. fetch returns Promise - multiple requests
  let r1 = await fetch("https://httpbin.org/get");
  let r2 = await fetch("https://httpbin.org/headers");
  console.log("FETCH_1:", r1.status, "FETCH_2:", r2.status);

  // 4. Promise.all with fetch
  let urls = [
    "https://httpbin.org/get",
    "https://httpbin.org/uuid",
    "https://httpbin.org/headers",
  ];
  let results = await Promise.all(urls.map((u) => fetch(u)));
  let allOk = results.every((r) => r.ok);
  let count = results.length;
  console.log("PROMISE_ALL_FETCHES:", count, "all_ok:", allOk);

  // 5. fetchAll equivalent - same as Promise.all
  let parallel = await Promise.all(
    urls.map((u) => fetch(u).then((r) => ({ ok: r.ok })))
  );
  console.log(
    "FETCH_ALL_ASYNC:",
    parallel.length,
    "parallel_ok:",
    parallel.every((r) => r.ok)
  );

  console.log("MAIN_DONE");
  // Allow timers to fire before exit
  await new Promise((r) => setTimeout(r, 50));
  console.log("ASYNC_VALIDATION_END");
})();
