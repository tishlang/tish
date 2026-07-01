#!/usr/bin/env bash
# HTTP throughput benchmark — the multi-worker server path the single-shot core/*
# perf tests never exercise (where tish's prefork + the send-values value type earn
# their keep).
#
# Server and load generator are SEPARATE PROCESSES by construction. A throughput test
# must drive the port-holder from OUTSIDE its own event loop — never self-fetch. A
# script that opens a port AND fetches it in-process measures response-generation
# competing with load-generation on the same runtime, not the server's real
# request-handling capacity. So the server is one process and the load is another.
#
# Modes:
#   # 1) Two-process (recommended; cross-terminal / cross-host / CI):
#   scripts/run_http_perf.sh --serve tish [--workers N]    # process A: the server (blocks)
#   scripts/run_http_perf.sh --serve node [--workers N]    #   (node reference server)
#   scripts/run_http_perf.sh --serve bun  [--workers N]    #   (bun native Bun.serve server)
#   scripts/run_http_perf.sh --url http://127.0.0.1:8080   # process B: the external load
#
#   # 2) One-shot local comparison (orchestrates the two separate processes for you):
#   scripts/run_http_perf.sh                               # tish vs node vs bun, 1 vs N workers
#
# Requires: oha, jq, curl (always); node (compare / --serve node); a release tish
# binary (compare / --serve tish, unless --no-build). bun is OPTIONAL — its rows are
# added when `bun` is on PATH, skipped (with a note) otherwise.
#
# Flags: --duration 5s  --connections 128  --workers N  --port 8080  --no-build
set -uo pipefail
cd "$(dirname "$0")/.." || exit 1

DUR="5s"; CONN=128; PORT=8080; NO_BUILD=0
MODE="compare"; SERVE_ENGINE="tish"; URL=""; WORKERS=""
NCPU=$(sysctl -n hw.ncpu 2>/dev/null || nproc 2>/dev/null || echo 4)
TISH="${TISH_BIN:-target/release/tish}"
SERVER_TISH="tests/http/server.tish"
SERVER_NODE="tests/http/server.mjs"
SERVER_BUN="tests/http/server.bun.js"
BIN="/tmp/tish_http_perf_server"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --serve) MODE="serve"; SERVE_ENGINE="${2:-tish}"; shift 2 ;;
    --url) MODE="url"; URL="${2%/}"; shift 2 ;;
    --duration) DUR="$2"; shift 2 ;;
    --connections) CONN="$2"; shift 2 ;;
    --workers) WORKERS="$2"; shift 2 ;;
    --port) PORT="$2"; shift 2 ;;
    --no-build) NO_BUILD=1; shift ;;
    *) echo "unknown arg: $1"; exit 2 ;;
  esac
done
MULTI="${WORKERS:-$NCPU}"

need() { command -v "$1" >/dev/null 2>&1 || { echo "missing required tool: $1"; exit 1; }; }

build_tish_server() {
  if [[ "$NO_BUILD" -eq 1 ]]; then
    [[ -x "$BIN" ]] || { echo "no server binary at $BIN (drop --no-build)"; exit 1; }; return
  fi
  [[ -x "$TISH" ]] || { echo "no tish binary at $TISH (cargo build -p tishlang --release)"; exit 1; }
  echo "Building tish server (rust backend, release)..." >&2
  "$TISH" build "$SERVER_TISH" -o "$BIN" --target native --native-backend rust \
    --feature http --feature process >/tmp/tish_http_build.log 2>&1 \
    || { echo "build failed:"; tail -20 /tmp/tish_http_build.log; exit 1; }
}

# bench BASE PATH -> "rps<TAB>p50ms<TAB>p99ms<TAB>successRate"
bench() {
  oha --no-tui --output-format json -z "$DUR" -c "$CONN" "${1}${2}" 2>/dev/null \
  | jq -r '[ (.summary.requestsPerSec|floor),
             ((.latencyPercentiles.p50*1000*100|round)/100),
             ((.latencyPercentiles.p99*1000*100|round)/100),
             (.summary.successRate) ] | @tsv'
}
warmup()     { oha --no-tui --output-format json -z 1s -c "$CONN" "${1}${2}" >/dev/null 2>&1; }
wait_ready() { for _ in $(seq 1 100); do curl -s "${1}/plaintext" >/dev/null 2>&1 && return 0; sleep 0.1; done; return 1; }

macos_caveat() {
  [[ "$(uname -s)" == "Darwin" ]] || return 0
  cat >&2 <<'EOF'

NOTE (macOS): tish prefork uses SO_REUSEPORT, which does NOT kernel-load-balance on Darwin —
      every connection funnels to one worker, so multi-worker tish reads ~ the same as w=1.
      Scaling is real on Linux (the deployment target); the fair LOCAL comparison is w=1.
EOF
}

# ---------------------- mode: serve (the server process; blocks) ----------------------
if [[ "$MODE" == serve ]]; then
  if [[ "$SERVE_ENGINE" == node ]]; then
    need node
    echo "node server: http://127.0.0.1:$PORT  (WORKERS=$MULTI)  — Ctrl-C to stop" >&2
    exec env PORT="$PORT" WORKERS="$MULTI" node "$SERVER_NODE"
  elif [[ "$SERVE_ENGINE" == bun ]]; then
    need bun
    echo "bun server: http://127.0.0.1:$PORT  (WORKERS=$MULTI)  — Ctrl-C to stop" >&2
    exec env PORT="$PORT" WORKERS="$MULTI" bun "$SERVER_BUN"
  else
    build_tish_server; macos_caveat
    echo "tish server: http://127.0.0.1:$PORT  (TISH_HTTP_WORKERS=$MULTI)  — Ctrl-C to stop" >&2
    exec env PORT="$PORT" TISH_HTTP_WORKERS="$MULTI" "$BIN"
  fi
fi

# ---------------------- mode: url (the load process; external) ------------------------
if [[ "$MODE" == url ]]; then
  need oha; need jq; need curl
  [[ -n "$URL" ]] || { echo "--url requires a URL"; exit 2; }
  wait_ready "$URL" || { echo "no server reachable at $URL/plaintext"; exit 1; }
  echo "Load against $URL  (duration=$DUR connections=$CONN)"
  {
    printf 'endpoint\treq/s\tp50ms\tp99ms\tsuccess\n'
    for path in /plaintext /json; do
      warmup "$URL" "$path"
      printf '%s\t%s\n' "$path" "$(bench "$URL" "$path")"
    done
  } | column -t -s "$(printf '\t')"
  exit 0
fi

# ---------------------- mode: compare (orchestrate both processes) --------------------
need oha; need jq; need curl; need node
killsrv() { pkill -f tish_http_perf_server >/dev/null 2>&1; pkill -f "$SERVER_NODE" >/dev/null 2>&1; pkill -f "$SERVER_BUN" >/dev/null 2>&1; sleep 0.3; }
trap killsrv EXIT
killsrv
build_tish_server
BASE="http://127.0.0.1:$PORT"

ROWS=()
run_case() { # engine workers
  local engine="$1" workers="$2" label
  if [[ "$engine" == tish ]]; then
    label="tish (w=$workers)"; PORT="$PORT" TISH_HTTP_WORKERS="$workers" "$BIN" >/tmp/tish_http_srv.log 2>&1 &
  elif [[ "$engine" == bun ]]; then
    label="bun (w=$workers)"; PORT="$PORT" WORKERS="$workers" bun "$SERVER_BUN" >/tmp/tish_http_srv.log 2>&1 &
  else
    label="node (w=$workers)"; PORT="$PORT" WORKERS="$workers" node "$SERVER_NODE" >/tmp/tish_http_srv.log 2>&1 &
  fi
  if ! wait_ready "$BASE"; then echo "  ! $label failed to start:"; tail -5 /tmp/tish_http_srv.log; killsrv; return 1; fi
  local path line
  for path in /plaintext /json; do
    warmup "$BASE" "$path"
    line="$(bench "$BASE" "$path")"
    ROWS+=("${label}	${path}	${line}")
    printf '  %-14s %-11s %s\n' "$label" "$path" "$(printf '%s' "$line" | tr '\t' ' ')"
  done
  killsrv
}

macos_caveat
echo "HTTP throughput: tish vs node vs bun  (duration=$DUR connections=$CONN workers: 1 and $MULTI, ncpu=$NCPU)"
echo "  engine         endpoint    req/s   p50ms p99ms  success"
run_case tish 1
run_case tish "$MULTI"
run_case node 1
run_case node "$MULTI"
if command -v bun >/dev/null 2>&1; then
  run_case bun 1
  run_case bun "$MULTI"
else
  echo "  (bun not on PATH — skipping bun rows; install: curl -fsSL https://bun.sh/install | bash)"
fi

echo ""
echo "=================== HTTP throughput summary ==================="
{
  printf 'engine\tendpoint\treq/s\tp50ms\tp99ms\tsuccess\n'
  for r in "${ROWS[@]}"; do printf '%s\n' "$r"; done
} | column -t -s "$(printf '\t')"
echo "==============================================================="
macos_caveat
