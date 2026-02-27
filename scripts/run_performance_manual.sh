#!/usr/bin/env bash
# Run Tish and JS equivalents, show output and compare execution time.
# Usage: ./scripts/run_performance_manual.sh [--release] [--summary-only]
#   --release       use release build (recommended for fair Tish vs JS timing)
#   --summary-only  skip individual test output, show only summary

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
summary_only=false

# Parse arguments
while [[ $# -gt 0 ]]; do
  case "$1" in
    --release) profile="release"; shift ;;
    --summary-only) summary_only=true; shift ;;
    *) shift ;;
  esac
done

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

# Arrays to collect summary data
declare -a summary_names=()
declare -a summary_tish=()
declare -a summary_node=()
declare -a summary_bun=()
declare -a summary_ratio=()

echo "=== Tish vs JavaScript MVP — output + timing ==="
$has_bun && echo "(Bun detected: $("$bun_cmd" --version 2>/dev/null || echo 'unknown'))"
echo ""

for f in "$perf_dir"/*.js; do
  [[ -f "$f" ]] || continue
  base=$(basename "$f" .js)
  tish_file="$tish_dir/$base.tish"
  [[ -f "$tish_file" ]] || continue

  if ! $summary_only; then
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
  else
    echo -n "Running $base..."
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
  bun_avg=0
  ratio=0
  
  if $has_bun; then
    for t in "${bun_times[@]}"; do bun_sum=$((bun_sum + t)); done
    bun_avg=$((bun_sum / n))
  fi
  
  if [[ $node_avg -gt 0 ]]; then
    ratio=$((tish_avg * 100 / node_avg))
  fi

  # Store results for summary
  summary_names+=("$base")
  summary_tish+=("$tish_avg")
  summary_node+=("$node_avg")
  summary_bun+=("$bun_avg")
  summary_ratio+=("$ratio")

  if ! $summary_only; then
    if $has_bun; then
      echo "Time (${n} runs avg): Tish ${tish_avg}ms | Node ${node_avg}ms | Bun ${bun_avg}ms"
      echo "Tish/Node ratio: ${ratio}%"
      if [[ $bun_avg -gt 0 ]]; then
        bun_ratio=$((tish_avg * 100 / bun_avg))
        echo "Tish/Bun ratio: ${bun_ratio}%"
      fi
    else
      echo "Time (${n} runs avg): Tish ${tish_avg}ms | Node ${node_avg}ms"
      echo "Tish/Node ratio: ${ratio}%"
    fi
    echo ""
  else
    echo " done (Tish: ${tish_avg}ms, ratio: ${ratio}%)"
  fi
done

# Print summary sorted by Tish/Node ratio (highest/slowest first)
echo ""
echo "═══════════════════════════════════════════════════════════════════════════════"
echo "                         PERFORMANCE SUMMARY"
echo "                    (sorted by Tish/Node ratio, slowest first)"
echo "═══════════════════════════════════════════════════════════════════════════════"
echo ""

# Create sortable data and sort by ratio (descending)
sorted_indices=()
for i in "${!summary_ratio[@]}"; do
  sorted_indices+=("${summary_ratio[$i]}:$i")
done
IFS=$'\n' sorted_indices=($(sort -t: -k1 -nr <<<"${sorted_indices[*]}")); unset IFS

# Print header
if $has_bun; then
  printf "%-30s %10s %10s %10s %12s\n" "Test Name" "Tish (ms)" "Node (ms)" "Bun (ms)" "Tish/Node %"
  printf "%-30s %10s %10s %10s %12s\n" "─────────────────────────────" "──────────" "──────────" "──────────" "────────────"
else
  printf "%-30s %10s %10s %12s\n" "Test Name" "Tish (ms)" "Node (ms)" "Tish/Node %"
  printf "%-30s %10s %10s %12s\n" "─────────────────────────────" "──────────" "──────────" "────────────"
fi

# Print sorted results
total_tish=0
total_node=0
total_bun=0
for entry in "${sorted_indices[@]}"; do
  idx="${entry#*:}"
  name="${summary_names[$idx]}"
  tish="${summary_tish[$idx]}"
  node="${summary_node[$idx]}"
  bun="${summary_bun[$idx]}"
  ratio="${summary_ratio[$idx]}"
  
  total_tish=$((total_tish + tish))
  total_node=$((total_node + node))
  total_bun=$((total_bun + bun))
  
  # Color coding based on ratio
  color=""
  reset=""
  if [[ -t 1 ]]; then  # Only use colors if stdout is a terminal
    if [[ $ratio -gt 500 ]]; then
      color="\033[1;31m"  # Red for very slow (>5x)
    elif [[ $ratio -gt 200 ]]; then
      color="\033[0;33m"  # Yellow for slow (>2x)
    elif [[ $ratio -lt 150 ]]; then
      color="\033[0;32m"  # Green for good (<1.5x)
    fi
    reset="\033[0m"
  fi
  
  if $has_bun; then
    printf "${color}%-30s %10d %10d %10d %11d%%${reset}\n" "$name" "$tish" "$node" "$bun" "$ratio"
  else
    printf "${color}%-30s %10d %10d %11d%%${reset}\n" "$name" "$tish" "$node" "$ratio"
  fi
done

# Print totals
echo ""
if $has_bun; then
  printf "%-30s %10s %10s %10s %12s\n" "─────────────────────────────" "──────────" "──────────" "──────────" "────────────"
else
  printf "%-30s %10s %10s %12s\n" "─────────────────────────────" "──────────" "──────────" "────────────"
fi

if [[ $total_node -gt 0 ]]; then
  total_ratio=$((total_tish * 100 / total_node))
else
  total_ratio=0
fi

if $has_bun; then
  printf "%-30s %10d %10d %10d %11d%%\n" "TOTAL" "$total_tish" "$total_node" "$total_bun" "$total_ratio"
else
  printf "%-30s %10d %10d %11d%%\n" "TOTAL" "$total_tish" "$total_node" "$total_ratio"
fi

echo ""
echo "Legend: Green = <150% | Yellow = 200-500% | Red = >500%"
echo ""
echo "─────────────────────────────────────────"
echo "Done."
