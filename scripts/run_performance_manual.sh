#!/usr/bin/env bash
# Run Tish and JS equivalents, show output and compare execution time.
# Usage: ./scripts/run_performance_manual.sh [--release]
#   --release   use release build (recommended for fair Tish vs JS timing)

set -e
cd "$(dirname "$0")/.."
node_cmd="${NODE:-node}"
tish_dir="tests/mvp"
perf_dir="performance/mvp"
target_dir="${CARGO_TARGET_DIR:-$(pwd)/target}"
profile="debug"
[[ "${1:-}" == "--release" ]] && { profile="release"; shift; }
tish_bin="$target_dir/$profile/tish"
rel_flag=""
[[ "$profile" == "release" ]] && rel_flag="--release"
if [[ ! -x "$tish_bin" ]]; then
  echo "Building tish ($profile)..."
  cargo build -p tish $rel_flag -q 2>/dev/null || true
fi
if [[ ! -x "$tish_bin" ]]; then
  tish_bin="cargo run -p tish $rel_flag -q --"
fi

# Millisecond timer (using node, always available for this script)
ms() { "$node_cmd" -e 'console.log(Date.now())'; }

echo "=== Tish vs JavaScript MVP — output + timing ==="
echo ""

for f in "$perf_dir"/*.js; do
  [[ -f "$f" ]] || continue
  base=$(basename "$f" .js)
  tish_file="$tish_dir/$base.tish"
  [[ -f "$tish_file" ]] || continue

  echo "─────────────────────────────────────────"
  echo "▶ $base"
  echo "─────────────────────────────────────────"

  # Output
  echo "Tish:"
  $tish_bin run "$tish_file" 2>&1 || true
  echo ""
  echo "JS:"
  "$node_cmd" "$f" 2>&1 || true
  echo ""

  # Timing (multiple runs to reduce noise; report median or average)
  n=5
  tish_times=()
  js_times=()
  for _ in $(seq 1 "$n"); do
    t0=$(ms)
    $tish_bin run "$tish_file" >/dev/null 2>&1 || true
    t1=$(ms)
    tish_times+=($((t1 - t0)))
  done
  for _ in $(seq 1 "$n"); do
    t0=$(ms)
    "$node_cmd" "$f" >/dev/null 2>&1 || true
    t1=$(ms)
    js_times+=($((t1 - t0)))
  done

  tish_sum=0
  js_sum=0
  for t in "${tish_times[@]}"; do tish_sum=$((tish_sum + t)); done
  for t in "${js_times[@]}"; do js_sum=$((js_sum + t)); done
  tish_avg=$((tish_sum / n))
  js_avg=$((js_sum / n))

  echo "Time (${n} runs avg): Tish ${tish_avg}ms | JS ${js_avg}ms"
  if [[ $js_avg -gt 0 ]]; then
    ratio=$((tish_avg * 100 / js_avg))
    echo "Tish/JS ratio: ${ratio}%"
  fi
  echo ""
done

echo "─────────────────────────────────────────"
echo "Done."
