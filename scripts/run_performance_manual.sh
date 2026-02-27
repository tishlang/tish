#!/usr/bin/env bash
# Run Tish and JS equivalents, show output and compare execution time.
# Usage: ./scripts/run_performance_manual.sh [--release]
#   --release   use release build (recommended for fair Tish vs JS timing)

set -e
cd "$(dirname "$0")/.."
node_cmd="${NODE:-node}"
bun_cmd="${BUN:-bun}"
has_bun=false
command -v "$bun_cmd" &>/dev/null && has_bun=true

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
$has_bun && echo "(Bun detected: $("$bun_cmd" --version 2>/dev/null || echo 'unknown'))"
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
  echo "Node:"
  "$node_cmd" "$f" 2>&1 || true
  echo ""
  if $has_bun; then
    echo "Bun:"
    "$bun_cmd" "$f" 2>&1 || true
    echo ""
  fi

  # Timing (multiple runs to reduce noise; report median or average)
  n=5
  tish_times=()
  node_times=()
  bun_times=()
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
    node_times+=($((t1 - t0)))
  done
  if $has_bun; then
    for _ in $(seq 1 "$n"); do
      t0=$(ms)
      "$bun_cmd" "$f" >/dev/null 2>&1 || true
      t1=$(ms)
      bun_times+=($((t1 - t0)))
    done
  fi

  tish_sum=0
  node_sum=0
  bun_sum=0
  for t in "${tish_times[@]}"; do tish_sum=$((tish_sum + t)); done
  for t in "${node_times[@]}"; do node_sum=$((node_sum + t)); done
  tish_avg=$((tish_sum / n))
  node_avg=$((node_sum / n))
  
  if $has_bun; then
    for t in "${bun_times[@]}"; do bun_sum=$((bun_sum + t)); done
    bun_avg=$((bun_sum / n))
    echo "Time (${n} runs avg): Tish ${tish_avg}ms | Node ${node_avg}ms | Bun ${bun_avg}ms"
    if [[ $node_avg -gt 0 ]]; then
      ratio=$((tish_avg * 100 / node_avg))
      echo "Tish/Node ratio: ${ratio}%"
    fi
    if [[ $bun_avg -gt 0 ]]; then
      ratio=$((tish_avg * 100 / bun_avg))
      echo "Tish/Bun ratio: ${ratio}%"
    fi
  else
    echo "Time (${n} runs avg): Tish ${tish_avg}ms | Node ${node_avg}ms"
    if [[ $node_avg -gt 0 ]]; then
      ratio=$((tish_avg * 100 / node_avg))
      echo "Tish/Node ratio: ${ratio}%"
    fi
  fi
  echo ""
done

echo "─────────────────────────────────────────"
echo "Done."
