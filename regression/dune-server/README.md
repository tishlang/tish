# dune-server regression

Guards the tish primitives that **Dune's headless backend** (`apps/dune-server` in the
[dune-ide](https://github.com/duneyou/dune-ide) repo) depends on, so a tish release can't silently
break the mission-critical surface. Unlike `regression/examples` (which runs the JS interpreter),
this **builds natively** and drives the real native-only primitives — `tish:pty`, the HTTP→WS
upgrade + `tish:ws` `wsAccept`, and the hyper `serve()` backend — end-to-end.

Why it exists: Dune connects to a remote `dune-server` for files, git, an interactive terminal, and
live change-push. That server is one native tish binary built with
`--feature http,http-hyper,fs,process,ws,pty` and `TISH_HTTP_BACKEND=hyper`. Several of those
primitives (`wsAccept`, the HTTP→WS upgrade, `tish:pty`, `Promise.spawn`) are **native-target only**
and were added *for* Dune (tish #493/#494 pty, #495/#496 upgrade), so nothing else in the suite
exercises them. A break there would only surface when someone updates Dune's tish pin — weeks later.

## What it checks

`server.tish` is a **distilled dune-server** — self-contained (no dune-ide imports), reproducing the
exact usage pattern. `drive.mjs` (Node ≥ 21, global `WebSocket`) asserts, over a freshly-built native
binary:

| # | Surface | tish primitive |
|---|---|---|
| 1 | `GET /health` → `{"ok":true}` | `tish:http` `serve()` (hyper) |
| 2 | `POST /rpc` `ping` → `"pong"` | HTTP RPC dispatch |
| 3 | `workspace_signature` **changes** after a file edit | `tish:fs` `stat` + `readDir` + `isDir` |
| 4 | `git_head` → the branch name | `process.execFileCapture` |
| 5 | `/pty` WS spawns a shell + **echoes** a command | `tish:pty` over the HTTP→WS upgrade + `wsAccept` |
| 6 | `/watch` WS **pushes** a changed signature on edit | `Promise.spawn` pump + upgrade dispatch by first frame |

A red on any of these is a real regression in a primitive Dune ships on.

## Run it

```bash
regression/dune-server/run.sh              # build native + drive + assert (exit 0 = pass)
regression/dune-server/run.sh --tish DIR   # build with a different tish checkout's CLI
regression/dune-server/run.sh --keep       # keep the scratch workspace + binary
```

It resolves a `tish` CLI (the checkout's `target/{release,debug}/tish`, else one on `PATH`, else
builds release), makes a temp git workspace, native-builds `server.tish` with dune-server's exact
flags, starts it, waits for the port, then runs the driver. CI: `.github/workflows/dune-server-regression.yml`
(triggered on changes to `crates/tish_runtime/**` and this dir).

## Keeping it faithful

`server.tish` mirrors `apps/dune-server` in dune-ide (`serveWorkspace.tish` + `servePtyWs.tish`). If
Dune's server changes which primitives it leans on, update `server.tish` here to match — this test is
only as good as its fidelity to the real thing.

## Related: isolated per-primitive smoke tests

This end-to-end suite is complemented by **isolated** native tests for the individual primitives, wired
into `scripts/test_native_smoke.sh` (CI: `native-smoke.yml`) so each has its own standing guard:
`tests/native_smoke/pty_app.tish` (`tish:pty`), `ws_app.tish` (HTTP→WS upgrade + `wsAccept`), and
`stat_app.tish` (`tish:fs` `stat`/`readDir`). The real dune-server can also be built against tish HEAD
on a dev machine by adding a `local:` entry to `regression/downstream/repos.tsv` (a direct `tish build`
cmd, so the runner skips the monorepo npm install).
