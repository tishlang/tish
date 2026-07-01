#!/usr/bin/env bash
# CI entrypoint for tish's multi-worker HTTP path. Assumes the toolchain (cargo, node,
# oha, curl, jq; bun optional) is already installed — the workflow installs it.
#
# Two parts, deliberately separated:
#   1. CORRECTNESS GATES (hard-fail the job): multithread dispatch + shared-counter
#      regression. Correctness is pass/fail.
#   2. THROUGHPUT RECORD (logged, NEVER gates): tish vs node vs bun, w=1 vs w=N. Raw
#      numbers only — perf is recorded, not judged (same policy as scripts/perf_record.sh).
#      Throughput on a shared single box is load-gen-contended; treat it as a trend log,
#      not an absolute-multiplier proof (that needs a separate load-gen host).
#
# Env: HTTP_WORKERS (default nproc)  HTTP_DURATION (5s)  HTTP_CONNECTIONS (128)
#      HTTP_PERF_OUT (http-perf-record.txt)
set -uo pipefail
cd "$(dirname "$0")/.."

WORKERS="${HTTP_WORKERS:-$(nproc 2>/dev/null || echo 4)}"
DUR="${HTTP_DURATION:-5s}"
CONN="${HTTP_CONNECTIONS:-128}"
OUT="${HTTP_PERF_OUT:-http-perf-record.txt}"

echo "== build release tish =="
cargo build --release --bin tish || { echo "BUILD FAILED"; exit 1; }

echo "== GATE 1: concurrent_shared_state (multithread dispatch, send-values) =="
cargo test -p tishlang_vm --features send-values --test concurrent_shared_state -- --nocapture \
  || { echo "GATE 1 FAILED — thread dispatch regressed"; exit 1; }

echo "== GATE 2: shared-counter regression (test_http_concurrency.sh -n 8) =="
bash scripts/test_http_concurrency.sh -n 8 \
  || { echo "GATE 2 FAILED — shared-counter/concurrency regressed"; exit 1; }

echo "== RECORD: multi-worker throughput — tish vs node vs bun (w=1 vs w=$WORKERS) =="
{
  echo "# HTTP perf record — $(uname -srm) — $(nproc 2>/dev/null || echo '?') cores — workers 1,$WORKERS — dur=$DUR conn=$CONN"
  echo "# node $(node --version 2>&1) | bun $(command -v bun >/dev/null 2>&1 && bun --version || echo ABSENT) | oha $(oha --version 2>&1 | head -1)"
  echo "# NOTE: single-box — the load generator (oha) shares cores with the server, so"
  echo "#       absolute req/s is contended; the scaling SHAPE (w=N vs w=1) is the signal."
  bash scripts/run_http_perf.sh --duration "$DUR" --connections "$CONN" --workers "$WORKERS"
} 2>&1 | tee "$OUT"

echo "== wrote $OUT (throughput is LOGGED, not gated) =="
exit 0
