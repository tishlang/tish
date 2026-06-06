#!/usr/bin/env bash
# HTTP concurrency CORRECTNESS test (not throughput) — does the server handle requests in PARALLEL?
#
# Fires N concurrent requests at a handler that busy-holds ~150ms each, and reads concurrency off the
# wall clock: parallel => wall ≈ one hold; serialized => wall ≈ N*hold. Server + client run as
# separate processes.
#
# PLATFORM NOTE (measured): macOS serializes — BSD SO_REUSEPORT does not kernel-distribute accepts, so
# prefork funnels to one worker (docs/concurrency-model.md). Real handler parallelism is a Linux
# property (the deployment / TechEmpower target). So this test ASSERTS parallelism on Linux and is
# informational (always exit 0) on macOS, printing the observed serial behavior.
#
# Usage: scripts/test_http_concurrency.sh [-n N] [-p PORT]   (needs target/release/tish built, curl, python3)
set -uo pipefail
cd "$(dirname "$0")/.."

N=6
PORT=$(( (RANDOM % 2000) + 8200 ))
HOLD_MS=150
TISH=target/release/tish
while [[ $# -gt 0 ]]; do case "$1" in -n) N="$2"; shift 2;; -p) PORT="$2"; shift 2;; *) shift;; esac; done

command -v curl >/dev/null    || { echo "SKIP: curl not found"; exit 0; }
command -v python3 >/dev/null || { echo "SKIP: python3 not found"; exit 0; }
[[ -x "$TISH" ]] || { echo "FAIL: $TISH not built (cargo build --release -p tishlang)"; exit 1; }
now_ms() { python3 -c 'import time;print(int(time.time()*1000))'; }

PORT="$PORT" TISH_HTTP_WORKERS="$N" "$TISH" run tests/http/concurrency_server.tish >/tmp/conc_srv.out 2>&1 &
SRV=$!
# Kill the server (and any prefork children) by name. No bare `wait` — the server runs forever and
# prefork children aren't this shell's jobs, so `wait` would hang.
cleanup() { kill "$SRV" 2>/dev/null; pkill -f concurrency_server >/dev/null 2>&1; }
trap cleanup EXIT

ready=0
for _ in $(seq 1 100); do curl -s --max-time 2 "http://127.0.0.1:$PORT/health" >/dev/null 2>&1 && { ready=1; break; }; sleep 0.1; done
[[ "$ready" == 1 ]] || { echo "FAIL: server never came up"; cat /tmp/conc_srv.out; exit 1; }

start=$(now_ms)
for _ in $(seq 1 "$N"); do curl -s --max-time 10 "http://127.0.0.1:$PORT/slow" >/dev/null 2>&1 & done
wait
wall=$(( $(now_ms) - start ))
serial=$(( N * HOLD_MS ))

echo "N=$N  hold=${HOLD_MS}ms  serial≈${serial}ms  parallel≈${HOLD_MS}-$(( HOLD_MS * 2 ))ms"
echo "  observed batch wall = ${wall}ms"

parallel=0
[[ "$wall" -lt $(( serial / 2 )) ]] && parallel=1
os=$(uname -s)
if [[ "$parallel" == 1 ]]; then
  echo "PASS: handlers run concurrently (${wall}ms « ${serial}ms serial)"; exit 0
elif [[ "$os" == "Darwin" ]]; then
  echo "INFO (macOS, exit 0): serialized as expected — SO_REUSEPORT does not distribute on Darwin."
  echo "      Handler parallelism is validated on Linux (the deployment target). Not a regression here."
  exit 0
else
  echo "FAIL: handlers SERIALIZED on $os (${wall}ms ≈ ${serial}ms) — expected parallel via SO_REUSEPORT prefork"
  exit 1
fi
