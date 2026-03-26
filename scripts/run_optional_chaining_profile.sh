#!/usr/bin/env bash
# Profile each optional_chaining section to identify freeze/slow parts.
# Usage: ./scripts/run_optional_chaining_profile.sh [--backend vm|interp]
#
# If optional_chaining freezes during parity/perf/manual tests, run this to find
# which operation (nullish ??, optional ?.) causes it.

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
backend_args=""
[[ "$1" == "--backend" && -n "${2:-}" ]] && backend_args="--backend $2"

echo "=== Optional chaining section profiling ==="
echo ""

for f in tests/core/optional_chaining_*.tish; do
  [[ -f "$f" ]] || continue
  name=$(basename "$f")
  echo "─────────────────────────────────────────"
  echo "▶ $name"
  echo "─────────────────────────────────────────"
  { time $tish_bin run "$f" $backend_args 2>&1; } 2>&1 || true
  echo ""
done

echo "Done. If any section hung, that's the culprit."
