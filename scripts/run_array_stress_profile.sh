#!/usr/bin/env bash
# Profile each array_stress section to identify slow parts.
# Usage: ./scripts/run_array_stress_profile.sh

set -e
cd "$(dirname "$0")/.."
target_dir="$(pwd)/target"
profile="debug"
tish_bin="$target_dir/$profile/tish"
if [[ ! -x "$tish_bin" ]]; then
  echo "Building tish ($profile, full features)..."
  cargo build -p tishlang--features full --target-dir "$target_dir" -q 2>/dev/null || true
fi
[[ ! -x "$tish_bin" ]] && tish_bin="cargo run -q -p tishlang--features full --target-dir $target_dir --"

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
