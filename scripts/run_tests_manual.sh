#!/usr/bin/env bash
# Run all .tish MVP tests and show output for manual verification.
# Usage: ./scripts/run_tests_manual.sh [--native]

set -e
cd "$(dirname "$0")/.."
tish_bin="cargo run -p tish -q --"

core_dir="tests/core"
run_native=false
[[ "${1:-}" == "--native" ]] && run_native=true

echo "=== Tish manual test run ==="
echo ""

for f in "$core_dir"/*.tish; do
  [[ -f "$f" ]] || continue
  name=$(basename "$f")
  echo "─────────────────────────────────────────"
  echo "▶ $name (interpreter)"
  echo "─────────────────────────────────────────"
  $tish_bin run "$f" 2>&1 || true
  echo ""

  if $run_native; then
    out="/tmp/tish_manual_$(basename "$f" .tish)"
    if $tish_bin compile "$f" -o "$out" 2>/dev/null; then
      echo "▶ $name (native)"
      echo "─────────────────────────────────────────"
      "$out" 2>&1 || true
      rm -f "$out"
      echo ""
    fi
  fi
done

echo "─────────────────────────────────────────"
echo "Done."
