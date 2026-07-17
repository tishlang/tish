// Drives the distilled dune-server (server.tish) and asserts the mission-critical surface end-to-end:
//   1. HTTP  /health                       → {"ok":true}
//   2. HTTP  /rpc ping                     → "pong"
//   3. HTTP  /rpc workspace_signature      → a string that CHANGES after a file edit (fs stat/readDir)
//   4. HTTP  /rpc git_head                 → the branch name (process.execFileCapture)
//   5. WS    /pty  {t:"start"} then {t:"in"} echo cmd → the shell echoes it back (tish:pty over the upgrade)
//   6. WS    /watch {t:"watch"} → baseline sig, then a PUSHED changed sig after a file edit
//
// Usage: node drive.mjs <baseUrl> <workspaceDir>   (e.g. node drive.mjs http://127.0.0.1:8799 /tmp/ws)
// Exit 0 = all pass, 1 = a failure. Requires Node >= 21 (global WebSocket).

import { writeFileSync } from "node:fs";

const BASE = process.argv[2];
const WS = "ws://" + BASE.replace(/^https?:\/\//, "");
const WORKSPACE = process.argv[3];

let failures = 0;
function check(name, ok, detail) {
  console.log((ok ? "PASS " : "FAIL ") + name + (detail ? "  " + detail : ""));
  if (!ok) failures++;
}

async function rpc(method) {
  const r = await fetch(BASE + "/rpc", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ method }),
  });
  return r.json();
}

function wsOnce(path, onOpen, onMessage, timeoutMs) {
  return new Promise((resolve) => {
    const sock = new WebSocket(WS + path);
    sock.binaryType = "arraybuffer";
    let done = false;
    const finish = (v) => { if (!done) { done = true; try { sock.close(); } catch {} resolve(v); } };
    sock.onopen = () => onOpen(sock);
    sock.onmessage = (e) => {
      const data = typeof e.data === "string" ? e.data : Buffer.from(e.data).toString("utf8");
      const r = onMessage(data, sock, finish);
      if (r !== undefined) finish(r);
    };
    sock.onerror = (e) => finish({ error: String(e && e.message ? e.message : e) });
    setTimeout(() => finish({ timeout: true }), timeoutMs);
  });
}

async function main() {
  // 1. health
  const health = await fetch(BASE + "/health").then((r) => r.text()).catch((e) => "ERR:" + e);
  check("http /health", health.includes("\"ok\":true"), health);

  // 2. ping
  const ping = await rpc("ping");
  check("rpc ping", ping && ping.ok === true && ping.result === "pong", JSON.stringify(ping));

  // 3. workspace_signature changes on a file edit (fs stat/readDir walk)
  const sig1 = await rpc("workspace_signature");
  writeFileSync(WORKSPACE + "/edited.txt", "changed-" + Math.random().toString(36).slice(2));
  const sig2 = await rpc("workspace_signature");
  check(
    "rpc workspace_signature (fs stat/readDir) changes on edit",
    sig1 && sig2 && typeof sig1.result === "string" && sig1.result !== sig2.result,
    (sig1 && sig1.result) + " -> " + (sig2 && sig2.result)
  );

  // 4. git_head (process.execFileCapture)
  const gh = await rpc("git_head");
  check(
    "rpc git_head (process.execFileCapture)",
    gh && gh.ok === true && gh.result && typeof gh.result.stdout === "string" && gh.result.stdout.trim().length > 0,
    gh && gh.result && JSON.stringify(gh.result.stdout)
  );

  // 5. /pty WS: spawn a shell, echo a marker, expect it back (tish:pty over the HTTP→WS upgrade)
  const MARKER = "DUNE_PTY_OK_" + Math.floor(Math.random() * 1e6);
  const ptyRes = await wsOnce(
    "/pty",
    (sock) => sock.send(JSON.stringify({ t: "start", cols: 80, rows: 24 })),
    (data, sock, finish) => {
      if (!sock.__sent && data.length > 0) {
        sock.__sent = true;
        sock.send(JSON.stringify({ t: "in", d: "echo " + MARKER + "\n" }));
      }
      sock.__buf = (sock.__buf || "") + data;
      if (sock.__buf.includes(MARKER + "\r") || sock.__buf.split(MARKER).length > 2) return { ok: true };
    },
    5000
  );
  check("ws /pty terminal echoes a command (tish:pty + wsAccept upgrade)", ptyRes && ptyRes.ok === true, JSON.stringify(ptyRes).slice(0, 120));

  // 6. /watch WS: baseline sig, then a PUSHED changed sig after an edit (Promise.spawn pump)
  const watchRes = await wsOnce(
    "/watch",
    (sock) => sock.send(JSON.stringify({ t: "watch" })),
    (data, sock, finish) => {
      const m = JSON.parse(data);
      sock.__sigs = sock.__sigs || [];
      sock.__sigs.push(m.sig);
      if (sock.__sigs.length === 1) {
        setTimeout(() => writeFileSync(WORKSPACE + "/watched.txt", "w-" + Math.random().toString(36).slice(2)), 150);
      }
      if (sock.__sigs.length === 2) return { ok: sock.__sigs[0] !== sock.__sigs[1], sigs: sock.__sigs };
    },
    6000
  );
  check("ws /watch pushes a changed signature on edit (Promise.spawn pump)", watchRes && watchRes.ok === true, JSON.stringify(watchRes).slice(0, 120));

  console.log("\n" + (failures === 0 ? "ALL PASS" : failures + " FAILURE(S)"));
  process.exit(failures === 0 ? 0 : 1);
}

main().catch((e) => { console.error("driver error", e); process.exit(1); });
