#!/bin/bash
# Run Tish and JS equivalents, show output and compare execution time.
# Usage: ./scripts/run_performance_manual.sh [--release] [--summary-only] [--no-compile]
#   --release       use release build (recommended for fair Tish vs JS timing)
#   --summary-onlfy  skip individual test output, show only summary
#   --no-compile    skip compilation, use cached binaries from previous runs

set -e
cd "$(dirname "$0")/.."

# Runtime detection
node_cmd="${NODE:-node}"
bun_cmd="${BUN:-bun}"
deno_cmd="${DENO:-deno}"
qjs_cmd="${QJS:-qjs}"

has_bun=false
has_deno=false
has_qjs=false
command -v "$bun_cmd" &>/dev/null && has_bun=true
command -v "$deno_cmd" &>/dev/null && has_deno=true
command -v "$qjs_cmd" &>/dev/null && has_qjs=true

tish_dir="tests/mvp"
perf_dir="performance/mvp"
# Always use local target directory for consistent builds
target_dir="$(pwd)/target"
profile="debug"
summary_only=false
no_compile=false

# Parse arguments
while [[ $# -gt 0 ]]; do
  case "$1" in
    --release) profile="release"; shift ;;
    --summary-only) summary_only=true; shift ;;
    --no-compile) no_compile=true; shift ;;
    *) shift ;;
  esac
done

tish_bin="$target_dir/$profile/tish"
rel_flag=""
[[ "$profile" == "release" ]] && rel_flag="--release"
if [[ ! -x "$tish_bin" ]]; then
  echo "Building tish ($profile)..."
  cargo build -p tish $rel_flag --target-dir "$target_dir" -q 2>/dev/null || true
fi
if [[ ! -x "$tish_bin" ]]; then
  tish_bin="cargo run -p tish $rel_flag --target-dir $target_dir -q --"
fi

# Directory for compiled outputs - use cache or temp
cache_dir="$target_dir/perf-cache-$profile"
if $no_compile; then
  compile_dir="$cache_dir"
  if [[ ! -d "$compile_dir" ]]; then
    echo "Error: No cached binaries found at $compile_dir"
    echo "Run without --no-compile first to build the cache."
    exit 1
  fi
else
  compile_dir="$cache_dir"
  mkdir -p "$compile_dir"
fi

# Millisecond timer - prefer perl/python for lower overhead, fallback to node
if command -v perl &>/dev/null; then
  ms() { perl -MTime::HiRes=time -e 'printf "%d\n", time*1000'; }
elif command -v python3 &>/dev/null; then
  ms() { python3 -c 'import time; print(int(time.time()*1000))'; }
else
  # Fallback to node (has ~25ms overhead)
  ms() { "$node_cmd" -e 'console.log(Date.now())'; }
fi

# Arrays to collect summary data
declare -a summary_names=()
declare -a summary_tish_run=()
declare -a summary_tish_native=()
declare -a summary_node=()
declare -a summary_bun=()
declare -a summary_deno=()
declare -a summary_qjs=()
declare -a summary_ratio=()

# Track which tests compiled successfully (using files instead of associative array for bash 3 compat)

echo "=== Tish vs JavaScript Runtimes — Performance Comparison ==="
echo "Profile: $profile"
echo ""
echo "Detected runtimes:"
echo "  Node.js: $("$node_cmd" --version 2>/dev/null || echo 'not found')"
$has_bun && echo "  Bun: $("$bun_cmd" --version 2>/dev/null || echo 'unknown')"
$has_deno && echo "  Deno: $("$deno_cmd" --version 2>/dev/null | head -1 || echo 'unknown')"
$has_qjs && echo "  QuickJS: $(echo 'qjs' 2>/dev/null || echo 'detected')"
echo ""

# Phase 1: Pre-compile all native binaries (skip if --no-compile)
if $no_compile; then
  # Count existing cached binaries
  cached_count=$(find "$compile_dir" -name '*_native' -type f -perm +111 2>/dev/null | wc -l | tr -d ' ')
  echo "Using cached binaries from: $compile_dir"
  echo "Cached binaries: $cached_count"
  echo ""
else
  echo "Compiling native binaries..."
  compile_count=0
  compile_fail=0
  for f in "$perf_dir"/*.js; do
    [[ -f "$f" ]] || continue
    base=$(basename "$f" .js)
    tish_file="$tish_dir/$base.tish"
    [[ -f "$tish_file" ]] || continue
    
    native_bin="$compile_dir/${base}_native"
    echo -n "  $base... "
    if $tish_bin compile "$tish_file" -o "$native_bin" >/dev/null 2>&1; then
      echo "ok"
      compile_count=$((compile_count + 1))
    else
      echo "skip (unsupported features)"
      compile_fail=$((compile_fail + 1))
    fi
  done
  echo "Compiled: $compile_count, Skipped: $compile_fail"
  echo "Cache location: $compile_dir"
  echo ""
fi

# Phase 2: Run timing tests
echo "Running performance tests..."
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
    echo "Tish (run):"
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
    if $has_deno; then
      echo "Deno:"
      "$deno_cmd" run --allow-all "$f" 2>&1 || true
      echo ""
    fi
  else
    echo -n "Running $base..."
  fi

  # Timing (multiple runs to reduce noise; report average)
  #n=50
  n=5
  tish_run_times=()
  tish_native_times=()
  node_times=()
  bun_times=()
  deno_times=()
  qjs_times=()

  # Warmup runs (discard - warms disk cache and JIT)
  $tish_bin run "$tish_file" >/dev/null 2>&1 || true
  native_bin="$compile_dir/${base}_native"
  [[ -x "$native_bin" ]] && "$native_bin" >/dev/null 2>&1 || true
  "$node_cmd" "$f" >/dev/null 2>&1 || true
  $has_bun && "$bun_cmd" "$f" >/dev/null 2>&1 || true
  $has_deno && "$deno_cmd" run --allow-all "$f" >/dev/null 2>&1 || true
  $has_qjs && "$qjs_cmd" "$f" >/dev/null 2>&1 || true

  # Tish interpreter (run)
  for _ in $(seq 1 "$n"); do
    t0=$(ms)
    $tish_bin run "$tish_file" >/dev/null 2>&1 || true
    t1=$(ms)
    tish_run_times+=($((t1 - t0)))
  done

  # Tish native (use pre-compiled binary from warmup)
  compile_ok=false
  if [[ -x "$native_bin" ]]; then
    compile_ok=true
    for _ in $(seq 1 "$n"); do
      t0=$(ms)
      "$native_bin" >/dev/null 2>&1 || true
      t1=$(ms)
      tish_native_times+=($((t1 - t0)))
    done
  fi

  # Node.js
  for _ in $(seq 1 "$n"); do
    t0=$(ms)
    "$node_cmd" "$f" >/dev/null 2>&1 || true
    t1=$(ms)
    node_times+=($((t1 - t0)))
  done

  # Bun
  if $has_bun; then
    for _ in $(seq 1 "$n"); do
      t0=$(ms)
      "$bun_cmd" "$f" >/dev/null 2>&1 || true
      t1=$(ms)
      bun_times+=($((t1 - t0)))
    done
  fi

  # Deno
  if $has_deno; then
    for _ in $(seq 1 "$n"); do
      t0=$(ms)
      "$deno_cmd" run --allow-all "$f" >/dev/null 2>&1 || true
      t1=$(ms)
      deno_times+=($((t1 - t0)))
    done
  fi

  # QuickJS
  if $has_qjs; then
    for _ in $(seq 1 "$n"); do
      t0=$(ms)
      "$qjs_cmd" "$f" >/dev/null 2>&1 || true
      t1=$(ms)
      qjs_times+=($((t1 - t0)))
    done
  fi

  # Calculate averages
  tish_run_sum=0
  tish_native_sum=0
  node_sum=0
  bun_sum=0
  deno_sum=0
  qjs_sum=0

  for t in "${tish_run_times[@]}"; do tish_run_sum=$((tish_run_sum + t)); done
  for t in "${tish_native_times[@]}"; do tish_native_sum=$((tish_native_sum + t)); done
  for t in "${node_times[@]}"; do node_sum=$((node_sum + t)); done

  tish_run_avg=$((tish_run_sum / n))
  tish_native_avg=0
  $compile_ok && tish_native_avg=$((tish_native_sum / n))
  node_avg=$((node_sum / n))
  bun_avg=0
  deno_avg=0
  qjs_avg=0
  ratio=0

  if $has_bun; then
    for t in "${bun_times[@]}"; do bun_sum=$((bun_sum + t)); done
    bun_avg=$((bun_sum / n))
  fi

  if $has_deno; then
    for t in "${deno_times[@]}"; do deno_sum=$((deno_sum + t)); done
    deno_avg=$((deno_sum / n))
  fi

  if $has_qjs; then
    for t in "${qjs_times[@]}"; do qjs_sum=$((qjs_sum + t)); done
    qjs_avg=$((qjs_sum / n))
  fi

  if [[ $node_avg -gt 0 ]]; then
    ratio=$((tish_run_avg * 100 / node_avg))
  fi

  # Store results for summary
  summary_names+=("$base")
  summary_tish_run+=("$tish_run_avg")
  summary_tish_native+=("$tish_native_avg")
  summary_node+=("$node_avg")
  summary_bun+=("$bun_avg")
  summary_deno+=("$deno_avg")
  summary_qjs+=("$qjs_avg")
  summary_ratio+=("$ratio")

  if ! $summary_only; then
    echo "Time (${n} runs avg):"
    echo "  Tish (run):    ${tish_run_avg}ms"
    $compile_ok && echo "  Tish (native): ${tish_native_avg}ms"
    echo "  Node.js:       ${node_avg}ms"
    $has_bun && echo "  Bun:           ${bun_avg}ms"
    $has_deno && echo "  Deno:          ${deno_avg}ms"
    $has_qjs && echo "  QuickJS:       ${qjs_avg}ms"
    echo "  Tish(run)/Node ratio: ${ratio}%"
    echo ""
  else
    echo " done (run: ${tish_run_avg}ms, native: ${tish_native_avg}ms, ratio: ${ratio}%)"
  fi
done

# Print summary sorted by Tish(run)/Node ratio (highest/slowest first)
echo ""
echo "════════════════════════════════════════════════════════════════════════════════════════════════════════════════"
echo "                                           PERFORMANCE SUMMARY"
echo "                                    (sorted by Tish(run)/Node ratio, slowest first)"
echo "════════════════════════════════════════════════════════════════════════════════════════════════════════════════"
echo ""

# Create sortable data and sort by ratio (descending)
sorted_indices=()
for i in "${!summary_ratio[@]}"; do
  sorted_indices+=("${summary_ratio[$i]}:$i")
done
IFS=$'\n' sorted_indices=($(sort -t: -k1 -nr <<<"${sorted_indices[*]}")); unset IFS

# Build header dynamically based on available runtimes
header="%-25s %10s %10s %10s"
header_args=("Test Name" "Tish(run)" "Tish(nat)" "Node")
divider="─────────────────────────"
div_args=("──────────" "──────────" "──────────")

if $has_bun; then
  header="$header %10s"
  header_args+=("Bun")
  div_args+=("──────────")
fi
if $has_deno; then
  header="$header %10s"
  header_args+=("Deno")
  div_args+=("──────────")
fi
if $has_qjs; then
  header="$header %10s"
  header_args+=("QuickJS")
  div_args+=("──────────")
fi
header="$header %12s\n"
header_args+=("run/Node %")
div_args+=("────────────")

printf "$header" "${header_args[@]}"
printf "%-25s" "$divider"
for d in "${div_args[@]}"; do printf " %10s" "$d"; done
echo ""

# Print sorted results
total_tish_run=0
total_tish_native=0
total_node=0
total_bun=0
total_deno=0
total_qjs=0

for entry in "${sorted_indices[@]}"; do
  idx="${entry#*:}"
  name="${summary_names[$idx]}"
  tish_run="${summary_tish_run[$idx]}"
  tish_native="${summary_tish_native[$idx]}"
  node="${summary_node[$idx]}"
  bun="${summary_bun[$idx]}"
  deno="${summary_deno[$idx]}"
  qjs="${summary_qjs[$idx]}"
  ratio="${summary_ratio[$idx]}"

  total_tish_run=$((total_tish_run + tish_run))
  total_tish_native=$((total_tish_native + tish_native))
  total_node=$((total_node + node))
  total_bun=$((total_bun + bun))
  total_deno=$((total_deno + deno))
  total_qjs=$((total_qjs + qjs))

  # Color coding based on ratio
  color=""
  reset=""
  if [[ -t 1 ]]; then
    if [[ $ratio -gt 500 ]]; then
      color="\033[1;31m"  # Red for very slow (>5x)
    elif [[ $ratio -gt 200 ]]; then
      color="\033[0;33m"  # Yellow for slow (>2x)
    elif [[ $ratio -lt 150 ]]; then
      color="\033[0;32m"  # Green for good (<1.5x)
    fi
    reset="\033[0m"
  fi

  # Native display: show "-" if not compiled
  native_display="$tish_native"
  [[ $tish_native -eq 0 ]] && native_display="-"

  # Build row dynamically
  row="%-25s %10d %10s %10d"
  row_args=("$name" "$tish_run" "$native_display" "$node")

  if $has_bun; then
    row="$row %10d"
    row_args+=("$bun")
  fi
  if $has_deno; then
    row="$row %10d"
    row_args+=("$deno")
  fi
  if $has_qjs; then
    row="$row %10d"
    row_args+=("$qjs")
  fi
  row="$row %11d%%"
  row_args+=("$ratio")

  printf "${color}${row}${reset}\n" "${row_args[@]}"
done

# Print totals
echo ""
printf "%-25s" "$divider"
for d in "${div_args[@]}"; do printf " %10s" "$d"; done
echo ""

if [[ $total_node -gt 0 ]]; then
  total_ratio=$((total_tish_run * 100 / total_node))
else
  total_ratio=0
fi

native_total_display="$total_tish_native"
[[ $total_tish_native -eq 0 ]] && native_total_display="-"

row="%-25s %10d %10s %10d"
row_args=("TOTAL" "$total_tish_run" "$native_total_display" "$total_node")

if $has_bun; then
  row="$row %10d"
  row_args+=("$total_bun")
fi
if $has_deno; then
  row="$row %10d"
  row_args+=("$total_deno")
fi
if $has_qjs; then
  row="$row %10d"
  row_args+=("$total_qjs")
fi
row="$row %11d%%"
row_args+=("$total_ratio")

printf "$row\n" "${row_args[@]}"

echo ""
echo "Legend: Green = <150% | Yellow = 200-500% | Red = >500%"
echo "        Tish(run) = interpreter | Tish(nat) = compiled native"
echo ""
echo "─────────────────────────────────────────"
echo "Done."
