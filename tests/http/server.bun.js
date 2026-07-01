// Bun reference server for scripts/run_http_perf.sh — Bun.serve (bun's native fast
// path; running node:http under bun would understate it). Same two routes as
// server.tish / server.mjs. JSON is stringified per request to match the tish handler.
//
//   PORT     listen port (default 8080)
//   WORKERS  worker processes (default 1; >1 prefork via SO_REUSEPORT to match tish)
//
// Multi-worker model mirrors tish's prefork: the primary spawns WORKERS child bun
// processes, each binding the port with reusePort:true so the kernel load-balances.
const port = process.env.PORT ? parseInt(process.env.PORT) : 8080
const workers = process.env.WORKERS ? parseInt(process.env.WORKERS) : 1

function serve() {
  Bun.serve({
    port,
    reusePort: true,
    fetch(req) {
      const path = new URL(req.url).pathname
      if (path === '/plaintext')
        return new Response('Hello, World!', { headers: { 'Content-Type': 'text/plain' } })
      if (path === '/json')
        return new Response(JSON.stringify({ message: 'Hello, World!' }), { headers: { 'Content-Type': 'application/json' } })
      return new Response('Not Found', { status: 404 })
    },
  })
}

if (workers > 1 && !process.env.__BUN_WORKER) {
  // Primary: fork WORKERS children (each serves with reusePort), then wait on them.
  const procs = []
  for (let i = 0; i < workers; i++) {
    procs.push(Bun.spawn([process.execPath, import.meta.path], {
      env: { ...process.env, __BUN_WORKER: '1' },
      stdout: 'inherit',
      stderr: 'inherit',
    }))
  }
  await Promise.all(procs.map((p) => p.exited))
} else {
  serve()
}
