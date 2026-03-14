#!/usr/bin/env bash
# Profile each array_stress section to identify slow parts.
# Usage: ./scripts/run_array_stress_profile.sh

set -e
cd "$(dirname "$0")/.."
tish_bin="cargo run -q -p tish --features full --"

echo "=== Array stress section profiling ==="
echo ""

for f in tests/core/array_stress_*.tish; do
  [[ -f "$f" ]] || continue
  name=$(basename "$f")
  echo "─────────────────────────────────────────"
  echo "▶ $name"
  echo "─────────────────────────────────────────"
  { time $tish_bin run "$f" 2>&1; } 2>&1 || true
  echo ""
done

echo "Done. Compare timings to find the slow section."
