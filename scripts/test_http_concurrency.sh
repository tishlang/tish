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
cd "$(dirname "$0")/.." || exit 1

N=6
PORT=$(( (RANDOM % 2000) + 8200 ))
HOLD_MS=150
TISH=target/release/tish
while [[ $# -gt 0 ]]; do case "$1" in -n) N="$2"; shift 2;; -p) PORT="$2"; shift 2;; *) shift;; esac; done

command -v curl >/dev/null    || { echo "SKIP: curl not found"; exit 0; }
command -v python3 >/dev/null || { echo "SKIP: python3 not found"; exit 0; }
[[ -x "$TISH" ]] || { echo "FAIL: $TISH not built (cargo build --release -p tishlang)"; exit 1; }
now_ms() { python3 -c 'import time;print(int(time.time()*1000))'; }

# --- shared-counter regression --------------------------------------------------------------------
# The reported bug: a handler that mutates a module-level `let` (request counter / cache / rate-limiter)
# hangs under concurrent requests. It does not — concurrent handlers can safely read-modify-write shared
# module state (proven cross-platform by crates/tish_vm/tests/concurrent_shared_state.rs). This end-to-end
# check requires every concurrent request to return 200 and the server to stay responsive afterward. Run
# threaded (PREFORK=0) so the counter is shared across worker threads in one process (the contended path);
# a true deadlock would leave curls hanging until --max-time and report 000 instead of 200.
run_counter_regression() {
  local port=$(( (RANDOM % 2000) + 8400 )) out=/tmp/conc_counter.out srv ready=0 _ codes ok served
  PORT="$port" TISH_HTTP_PREFORK=0 TISH_HTTP_WORKERS="$N" "$TISH" run tests/http/concurrency_counter_server.tish >"$out" 2>&1 &
  srv=$!
  kill_counter() { kill "$srv" 2>/dev/null; pkill -f concurrency_counter_server >/dev/null 2>&1; }
  for _ in $(seq 1 100); do curl -s --max-time 2 "http://127.0.0.1:$port/health" >/dev/null 2>&1 && { ready=1; break; }; sleep 0.1; done
  [[ "$ready" == 1 ]] || { echo "FAIL: counter server never came up"; cat "$out"; kill_counter; return 1; }
  codes=$(for _ in $(seq 1 "$N"); do curl -s -o /dev/null --max-time 10 -w "%{http_code}\n" "http://127.0.0.1:$port/slow" & done; wait)
  ok=$(echo "$codes" | grep -c '^200$')
  served=$(curl -s -D - -o /dev/null --max-time 5 "http://127.0.0.1:$port/done" 2>/dev/null | awk -F': ' 'tolower($1)=="x-served"{gsub(/\r/,"",$2);print $2}')
  kill_counter
  echo "shared-counter: ${ok}/${N} concurrent /slow returned 200; served counter=${served:-<none>}"
  [[ "$ok" == "$N" ]] || { echo "FAIL (deadlock regression): shared-counter handler hung under concurrency (${ok}/${N} got 200; codes: $(echo "$codes" | tr '\n' ' '))"; return 1; }
  { [[ "$served" =~ ^[0-9]+$ ]] && [[ "$served" -gt 0 ]]; } || { echo "FAIL: implausible served count '${served:-<none>}'"; return 1; }
  echo "PASS: shared-counter handler stayed responsive under ${N} concurrent requests (no deadlock)"
  return 0
}
run_counter_regression || exit 1
echo "---"

PORT="$PORT" TISH_HTTP_WORKERS="$N" "$TISH" run tests/http/concurrency_server.tish >/tmp/conc_srv.out 2>&1 &
SRV=$!
# Kill the server (and any prefork children) by name. No bare `wait` — the server runs forever and
# prefork children aren't this shell's jobs, so `wait` would hang.
cleanup() { kill "$SRV" 2>/dev/null; pkill -f concurrency_server >/dev/null 2>&1; }
trap cleanup EXIT

ready=0
for _ in $(seq 1 100); do curl -s --max-time 2 "http://127.0.0.1:$PORT/health" >/dev/null 2>&1 && { ready=1; break; }; sleep 0.1; done
[[ "$ready" == 1 ]] || { echo "FAIL: server never came up"; cat /tmp/conc_srv.out; exit 1; }

serial=$(( N * HOLD_MS ))
# Parallelism, read off the wall clock. A single batch on a loaded SHARED runner can
# straddle the threshold: the kernel hashes SO_REUSEPORT connections to workers (N
# connections rarely land on N distinct workers) and scheduler jitter adds noise. Take the
# BEST of several batches — genuine parallelism clears the bar on at least one; a real
# serialization regression stays ≈ serial on ALL attempts. Early-exit once it passes.
best=2147483647
for attempt in 1 2 3 4 5; do
  start=$(now_ms)
  # Wait only on the curl PIDs — a bare `wait` would also block on the forever-running server.
  pids=()
  for _ in $(seq 1 "$N"); do curl -s --max-time 10 "http://127.0.0.1:$PORT/slow" >/dev/null 2>&1 & pids+=($!); done
  wait "${pids[@]}"
  wall=$(( $(now_ms) - start ))
  echo "  batch $attempt wall = ${wall}ms"
  [[ "$wall" -lt "$best" ]] && best=$wall
  [[ "$best" -lt $(( serial / 2 )) ]] && break
done
wall=$best

echo "N=$N  hold=${HOLD_MS}ms  serial≈${serial}ms  parallel≈${HOLD_MS}-$(( HOLD_MS * 2 ))ms"
echo "  best batch wall = ${wall}ms"

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
