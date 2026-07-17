#!/usr/bin/env bash
# Native-app hardening smoke test — builds real tish programs to native Rust binaries and asserts they
# actually RUN correctly end to end. Guards the classes of breakage seen recently (issue #469: native
# `serve` silently exiting; auto-invoked `main`; feature-gated globals in a `main` closure):
#
#   1. CLI app   — recursion, array/string builtins, process.env, `fn main()` entry, clean exit.
#   2. fs app    — write a file, read it back, JSON round-trip.
#   3. HTTP serve — both backends (tiny_http + hyper) via scripts/test_serve_smoke.sh.
#
# Run locally: cargo build --release -p tishlang --bin tish && scripts/test_native_smoke.sh
# Exits non-zero on ANY failure. Wired into CI (.github/workflows/native-smoke.yml).
set -uo pipefail
cd "$(dirname "$0")/.." || exit 1

TISH=${TISH:-target/release/tish}
FAIL=0

if [[ ! -x "$TISH" ]]; then
  echo "ERROR: $TISH not found — build it first (cargo build --release -p tishlang --bin tish)"
  exit 2
fi

# build_run NAME FIXTURE "<build feature flags>" "<run env>" "<expected stdout>"
build_run() {
  local name="$1" fixture="$2" run_env="$3" expected="$4"; shift 4
  local -a flags=("$@")   # feature flags as an array — properly quoted, no word-split
  local bin="target/native_smoke_${name}"
  echo "──────── ${name} (${fixture}) ────────"
  if ! "$TISH" build "$fixture" -o "$bin" --target native --native-backend rust "${flags[@]}" >"$bin.build.log" 2>&1; then
    echo "  ✗ BUILD FAILED"; tail -30 "$bin.build.log"; FAIL=1; return
  fi
  local got rc
  if [[ -n "$run_env" ]]; then
    got=$(env "$run_env" "$bin" 2>&1); rc=$?   # run_env is a single KEY=VAL
  else
    got=$("$bin" 2>&1); rc=$?
  fi
  if [[ "$rc" != 0 ]]; then
    echo "  ✗ NON-ZERO EXIT ($rc)"; echo "$got"; FAIL=1
  fi
  if [[ "$got" == "$expected" ]]; then
    echo "  ✓ output as expected (exit ${rc})"
  else
    echo "  ✗ OUTPUT MISMATCH"; echo "  --- got ---"; echo "$got"; echo "  --- want ---"; echo "$expected"; FAIL=1
  fi
  rm -f "$bin" "$bin.build.log"
}

# 1. CLI app
build_run "cli" "tests/native_smoke/cli_app.tish" "SMOKE_NAME=ci" \
"sorted: 1,2,3,5,8,9
sum: 28
fib10: 55
upper: NATIVE
hello: ci" \
--feature process

# 2. fs app
FSTMP="target/native_smoke_fs_$$.json"
build_run "fs" "tests/native_smoke/fs_app.tish" "SMOKE_FILE=$FSTMP" \
"fs-version: 3
fs-count: 3
fs-join: alpha-beta-gamma" \
--feature fs --feature process
rm -f "$FSTMP"

# 3. pty app — tish:pty (Dune's remote terminal). Spawn a shell, run a command, read its OUTPUT back.
build_run "pty" "tests/native_smoke/pty_app.tish" "" \
"pty: ok" \
--feature pty

# 4. fs stat/readDir app — the workspace-change signature walk. Deterministic count + total size.
STATDIR="target/native_smoke_stat_$$"
rm -rf "$STATDIR"
build_run "stat" "tests/native_smoke/stat_app.tish" "SMOKE_DIR=$STATDIR" \
"stat: count=2 size=15 isdir=true" \
--feature fs --feature process
rm -rf "$STATDIR"

# 5. HTTP→WS upgrade + wsAccept (issue #495/#496) — the transport Dune's /pty + /watch ride. Built with
# the hyper backend; a serve()+wsAccept echo run on Promise.spawn threads while the main thread connects
# as a WS client and asserts the echo. Its own inline check: the server logs a startup line, so this is a
# CONTAINS check ("ws: ok" present, "ws: FAIL" absent), and TISH_HTTP_PREFORK=0 keeps it single-process so
# the in-process client isn't forked.
echo "──────── ws-upgrade (tests/native_smoke/ws_app.tish) ────────"
WSBIN="target/native_smoke_ws"
if ! TISH_HTTP_BACKEND=hyper "$TISH" build tests/native_smoke/ws_app.tish -o "$WSBIN" \
      --target native --native-backend rust --feature http --feature http-hyper --feature ws \
      >"$WSBIN.build.log" 2>&1; then
  echo "  ✗ BUILD FAILED"; tail -30 "$WSBIN.build.log"; FAIL=1
else
  WSOUT=$(TISH_HTTP_BACKEND=hyper TISH_HTTP_PREFORK=0 "$WSBIN" 2>/dev/null)
  if echo "$WSOUT" | grep -q "ws: ok" && ! echo "$WSOUT" | grep -q "ws: FAIL"; then
    echo "  ✓ ws upgrade round-trip (client → serve upgrade → wsAccept echo → client)"
  else
    echo "  ✗ ws upgrade FAILED"; echo "$WSOUT"; FAIL=1
  fi
fi
rm -f "$WSBIN" "$WSBIN.build.log"

# 6. HTTP serve — both backends (delegates to the serve smoke).
echo "──────── http serve (both backends) ────────"
if TISH="$TISH" bash scripts/test_serve_smoke.sh; then
  echo "  ✓ serve smoke passed"
else
  echo "  ✗ serve smoke FAILED"; FAIL=1
fi

echo "════════════════════════════════"
if [[ "$FAIL" == 0 ]]; then
  echo "ALL NATIVE SMOKE TESTS PASSED (cli + fs + pty + stat + ws + http×2)"
else
  echo "NATIVE SMOKE FAILURES — see above"
fi
exit "$FAIL"
