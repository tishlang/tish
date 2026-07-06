#!/usr/bin/env bash
# Serve correctness smoke test — builds a native tish HTTP server and validates BOTH backends:
#   * tiny_http (default)
#   * hyper     (--feature http-hyper + TISH_HTTP_BACKEND=hyper)
#
# For each backend: build the native binary, start it, wait for the port to bind, assert a static
# route, a JSON route, and a dynamic (VM-dispatched) route, confirm the process stays alive, then
# shut it down. Exits non-zero on ANY failure. This is the standing CI guard that `serve` actually
# binds and serves on both paths (issue #469 — native serve was silently exiting).
#
# Usage: scripts/test_serve_smoke.sh          (needs a release `tish` built, curl)
set -uo pipefail
cd "$(dirname "$0")/.." || exit 1

TISH=${TISH:-target/release/tish}
FIX=tests/http/serve_smoke.tish
FAIL=0

if [[ ! -x "$TISH" ]]; then
  echo "ERROR: $TISH not found — build it first (cargo build --release -p tishlang --bin tish)"
  exit 2
fi

# test_backend NAME "<extra build feature flags>" "<runtime env, e.g. TISH_HTTP_BACKEND=hyper>"
test_backend() {
  local name="$1" run_env="$2"; shift 2
  local -a extra_flags=("$@")   # e.g. (--feature http-hyper --feature process) — array, no word-split
  local bin="target/serve_smoke_${name}"
  local log="target/serve_smoke_${name}.log"
  echo "──────── backend: ${name} ────────"

  echo "  build: tish build ... --feature http ${extra_flags[*]}"
  if ! "$TISH" build "$FIX" -o "$bin" --target native --native-backend rust --feature http "${extra_flags[@]}" >"$log.build" 2>&1; then
    echo "  ✗ BUILD FAILED"; tail -30 "$log.build"; FAIL=1; return
  fi

  local port=$(( (RANDOM % 2000) + 8300 ))
  echo "  serve on :${port} (${run_env:-default})"
  if [[ -n "$run_env" ]]; then
    env PORT="$port" "$run_env" "$bin" >"$log" 2>&1 &   # run_env is a single KEY=VAL
  else
    env PORT="$port" "$bin" >"$log" 2>&1 &
  fi
  local pid=$!

  # Wait up to ~10s for the port to accept connections.
  local up=0
  for _ in $(seq 1 100); do
    curl -sf "http://127.0.0.1:${port}/plaintext" >/dev/null 2>&1 && { up=1; break; }
    kill -0 "$pid" 2>/dev/null || break
    sleep 0.1
  done
  if [[ "$up" != 1 ]]; then
    echo "  ✗ SERVER NEVER BOUND :${port}"; echo "  --- server log ---"; cat "$log"
    kill "$pid" 2>/dev/null; wait "$pid" 2>/dev/null; FAIL=1; return
  fi

  check() {
    local path="$1" want="$2"
    local got; got=$(curl -s "http://127.0.0.1:${port}${path}")
    if [[ "$got" == "$want" ]]; then
      echo "  ✓ ${path} → '${got}'"
    else
      echo "  ✗ ${path}: got '${got}' want '${want}'"; FAIL=1
    fi
  }
  check "/plaintext" "Hello, World!"
  check "/json"      '{"message":"Hello, World!"}'
  check "/foo/bar"   "echo:/foo/bar"

  if kill -0 "$pid" 2>/dev/null; then
    echo "  ✓ process still serving"
  else
    echo "  ✗ process exited during test"; cat "$log"; FAIL=1
  fi

  kill "$pid" 2>/dev/null; wait "$pid" 2>/dev/null
}

# `--feature process` is included because a realistic server reads its port from `process.env`
# (the fixture does), and it exercises the process global captured into the `main` closure.
test_backend "tiny_http" ""                       --feature process
test_backend "hyper"     "TISH_HTTP_BACKEND=hyper" --feature http-hyper --feature process

echo "────────────────────────────────"
if [[ "$FAIL" == 0 ]]; then
  echo "ALL SERVE SMOKE TESTS PASSED (both http backends)"
else
  echo "SERVE SMOKE FAILURES — see above"
fi
exit "$FAIL"
