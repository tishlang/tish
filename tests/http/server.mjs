// Node reference server for scripts/run_http_perf.sh — plain `node:http` (no
// framework, the fairest baseline) with the same two routes as server.tish.
// JSON is stringified per request to match the tish handler exactly.
//
//   PORT     listen port (default 8080)
//   WORKERS  cluster workers (default 1; >1 forks to match tish multi-worker)
import http from 'node:http'
import cluster from 'node:cluster'

const port = process.env.PORT ? parseInt(process.env.PORT) : 8080
const workers = process.env.WORKERS ? parseInt(process.env.WORKERS) : 1

function start() {
    http.createServer((req, res) => {
        if (req.url === '/plaintext') {
            res.writeHead(200, { 'Content-Type': 'text/plain' })
            res.end('Hello, World!')
        } else if (req.url === '/json') {
            res.writeHead(200, { 'Content-Type': 'application/json' })
            res.end(JSON.stringify({ message: 'Hello, World!' }))
        } else {
            res.writeHead(404)
            res.end('Not Found')
        }
    }).listen(port)
}

if (workers > 1 && cluster.isPrimary) {
    for (let i = 0; i < workers; i++) cluster.fork()
} else {
    start()
}
