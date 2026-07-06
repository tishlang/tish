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

# 3. HTTP serve — both backends (delegates to the serve smoke).
echo "──────── http serve (both backends) ────────"
if TISH="$TISH" bash scripts/test_serve_smoke.sh; then
  echo "  ✓ serve smoke passed"
else
  echo "  ✗ serve smoke FAILED"; FAIL=1
fi

echo "════════════════════════════════"
if [[ "$FAIL" == 0 ]]; then
  echo "ALL NATIVE SMOKE TESTS PASSED (cli + fs + http×2)"
else
  echo "NATIVE SMOKE FAILURES — see above"
fi
exit "$FAIL"
