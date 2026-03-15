// HTTP Server Test - Node.js/Deno equivalent of http_server.tish
// Starts server, handles a request, then exits (for perf script compatibility)

import * as http from "node:http";

const PORT = 3001;

const server = http.createServer((req, res) => {
  let body = "";
  req.on("data", (chunk) => (body += chunk));
  req.on("end", () => {
    console.log("Request:", req.method, req.url);
    if (req.url === "/") {
      res.writeHead(200, { "Content-Type": "text/plain" });
      res.end("Hello from Tish HTTP Server!");
    } else if (req.url === "/json") {
      res.writeHead(200, { "Content-Type": "application/json" });
      res.end(JSON.stringify({ message: "Hello JSON", timestamp: 12345 }));
    } else if (req.url === "/echo") {
      res.writeHead(200, { "Content-Type": "text/plain" });
      res.end("Echo: " + body);
    } else {
      res.writeHead(404, { "Content-Type": "text/plain" });
      res.end("Not Found: " + req.url);
    }
  });
});

server.listen(PORT, "127.0.0.1", () => {
  fetch("http://127.0.0.1:" + PORT + "/")
    .then((r) => r.text())
    .then(() => {
      server.close();
    });
});
