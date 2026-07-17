#!/usr/bin/env bash
# ---------------------------------------------------------------------------
# dune-server regression — build a DISTILLED dune-server (server.tish) NATIVELY,
# exactly as Dune's headless backend does, then drive it end-to-end and assert the
# mission-critical primitives still work: tish:http serve() on the hyper backend,
# the HTTP→WS upgrade + tish:ws wsAccept, tish:pty, tish:fs stat/readDir, process,
# and Promise.spawn.
#
# The built-in example/downstream suites run the JS interpreter (`tish run`), which
# CAN'T exercise these native-only primitives — hence this dedicated native build+drive.
#
# Usage: regression/dune-server/run.sh [--tish DIR] [--keep]
#   --tish DIR   tish checkout to build with (default: this repo). Its `tish` CLI is used.
#   --keep       keep the scratch workspace + binary for inspection.
# Env: TISH_DIR overrides --tish.
# Exit 0 = all mission-critical checks pass; 1 = a regression.
# ---------------------------------------------------------------------------
set -uo pipefail

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
TISH="${TISH_DIR:-$(cd "$SCRIPT_DIR/../.." && pwd)}"
KEEP=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --tish) TISH=$(cd "$2" && pwd); shift 2 ;;
    --keep) KEEP=1; shift ;;
    *) echo "unknown option: $1" >&2; exit 2 ;;
  esac
done

# Resolve a `tish` CLI: prefer the checkout's built binary, else one on PATH.
TISH_BIN=""
for cand in "$TISH/target/release/tish" "$TISH/target/debug/tish" "$(command -v tish 2>/dev/null || true)"; do
  [[ -n "$cand" && -x "$cand" ]] && { TISH_BIN="$cand"; break; }
done
if [[ -z "$TISH_BIN" ]]; then
  echo "building tish CLI (release, --features full) from $TISH ..."
  ( cd "$TISH" && cargo build --release --features full ) || { echo "ERROR: could not build tish CLI"; exit 2; }
  TISH_BIN="$TISH/target/release/tish"
fi
command -v node >/dev/null 2>&1 || { echo "ERROR: node is required (the WS driver uses global WebSocket, Node >= 21)"; exit 2; }

WORK=$(mktemp -d "${TMPDIR:-/tmp}/dune-server-reg.XXXXXX")
WS="$WORK/ws"; mkdir -p "$WS"
BIN="$WORK/dune-server-parity"
SRV_PID=""
cleanup() {
  if [[ -n "$SRV_PID" ]]; then
    kill "$SRV_PID" 2>/dev/null
    wait "$SRV_PID" 2>/dev/null || true   # absorb the shell's "Terminated" job-control notice
  fi
  [[ $KEEP -eq 1 ]] || rm -rf "$WORK"
  [[ $KEEP -eq 1 ]] && echo "kept: $WORK"
}
trap cleanup EXIT

echo "tish:      $TISH_BIN ($(git -C "$TISH" rev-parse --short HEAD 2>/dev/null || echo '?'))"
echo "workspace: $WS"

# Seed the workspace (git repo so git_head has a branch; a couple files for the fs walk).
( cd "$WS" && git init -q && git config user.email t@t.co && git config user.name t \
    && printf 'hello\n' > a.txt && printf 'world\n' > sub_b.txt && git add -A && git commit -qm init ) \
  || { echo "ERROR: could not seed git workspace"; exit 2; }

# Build NATIVELY with the EXACT feature set + hyper backend dune-server uses.
echo "building server.tish → native (--feature http,http-hyper,fs,process,ws,pty) ..."
if ! TISH_HTTP_BACKEND=hyper "$TISH_BIN" build --target native \
      --feature http,http-hyper,fs,process,ws,pty \
      -o "$BIN" "$SCRIPT_DIR/server.tish" > "$WORK/build.log" 2>&1; then
  echo "FAIL: native build of the distilled dune-server did not compile"
  tail -30 "$WORK/build.log"
  exit 1
fi
echo "build OK"

# Pick a free-ish high port.
PORT=$(( 8800 + RANDOM % 800 ))

TISH_HTTP_BACKEND=hyper "$BIN" --workspace "$WS" --port "$PORT" > "$WORK/server.log" 2>&1 &
SRV_PID=$!

# Wait for the HTTP port to accept (up to ~5s).
up=0
for _ in $(seq 1 50); do
  if ! kill -0 "$SRV_PID" 2>/dev/null; then echo "FAIL: server exited early"; tail -20 "$WORK/server.log"; exit 1; fi
  if curl -s -m 1 "http://127.0.0.1:$PORT/health" >/dev/null 2>&1; then up=1; break; fi
  sleep 0.1
done
[[ $up -eq 1 ]] || { echo "FAIL: server never came up on $PORT"; tail -20 "$WORK/server.log"; exit 1; }

echo "server up on $PORT — driving …"
echo "---------------------------------------------"
if node "$SCRIPT_DIR/drive.mjs" "http://127.0.0.1:$PORT" "$WS"; then
  echo "---------------------------------------------"
  echo "dune-server regression: PASS"
  exit 0
else
  echo "---------------------------------------------"
  echo "dune-server regression: FAIL"
  echo "--- server.log tail ---"; tail -20 "$WORK/server.log"
  exit 1
fi
