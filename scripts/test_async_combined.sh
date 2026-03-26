#!/bin/bash
# Validate that async/await, Promise, and setTimeout work together.
# Requires network (httpbin.org). Run from repo root.
# Usage: ./scripts/test_async_combined.sh [--compile]

set -e
cd "$(dirname "$0")/.."

compile_native=false
[[ "${1:-}" == "--compile" ]] && compile_native=true

target_dir="${CARGO_TARGET_DIR:-$(pwd)/target}"
tish_bin="$target_dir/debug/tish"
script="tests/modules/async_promise_settimeout.tish"

# Build tish with http feature
echo "Building tish (http feature)..."
cargo build -p tishlang--features http --target-dir "$target_dir" -q 2>/dev/null || \
  cargo build -p tishlang--features http --target-dir "$target_dir"

if [[ ! -x "$tish_bin" ]]; then
  echo "Error: tish binary not found at $tish_bin"
  exit 1
fi

echo "Running combined async/Promise/setTimeout validation..."
output=$("$tish_bin" run "$script" --backend interp 2>&1) || true
exit_code=$?

echo "$output"
echo ""

# Validate expected output markers
fail=0
check() {
  if echo "$output" | grep -q "$1"; then
    echo "  ✓ $2"
  else
    echo "  ✗ MISSING: $2"
    fail=1
  fi
}

echo "Validation:"
check "ASYNC_VALIDATION_START"   "Script started"
check "PROMISE_AWAIT: ok"        "Promise + await"
check "FETCH_1:"                 "fetch request 1"
check "FETCH_2:"                 "fetch request 2"
check "PROMISE_ALL_FETCHES: 3"   "Promise.all with 3 fetches"
check "FETCH_ALL: 3"             "fetchAll parallel"
check "MAIN_DONE"                "Main execution completed"
check "TIMER_1_FIRED"            "setTimeout(0) callback ran"
check "TIMER_2_FIRED"            "setTimeout(20) callback ran"
check "ASYNC_VALIDATION_END"     "Script ended"

if [[ $fail -eq 1 ]]; then
  echo ""
  echo "Validation FAILED"
  exit 1
fi

# Optional: run compiled native binary
if $compile_native; then
  echo ""
  echo "Compiling to native..."
  native_out="$target_dir/async_combined_validation"
  "$tish_bin" compile "$script" -o "$native_out" || { echo "Compile failed"; exit 1; }
  echo "Running compiled binary..."
  output2=$("$native_out" 2>&1) || true
  echo "$output2"
  # Native: timers don't run; fetchAsync+Promise.all now works
  for m in "ASYNC_VALIDATION_START" "PROMISE_AWAIT" "MAIN_DONE" "ASYNC_VALIDATION_END"; do
    if echo "$output2" | grep -q "$m"; then
      echo "  ✓ Native: $m"
    else
      echo "  ✗ Native MISSING: $m"
      exit 1
    fi
  done
fi

echo ""
echo "All validations passed."
