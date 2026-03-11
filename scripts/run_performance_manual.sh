#!/bin/bash
# Run Tish and JS equivalents, show output and compare execution time.
# Usage: ./scripts/run_performance_manual.sh [--release] [--summary-only] [--no-compile] [--limit N]
#   --release       use release build (recommended for fair Tish vs JS timing)
#   --summary-only  skip individual test output, show only summary
#   --no-compile    skip compilation, use cached binaries from previous runs
#   --limit N       run only first N tests (default: all)
#   --timeout SEC   timeout per tish run in seconds (default: 30, 0=no limit)

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
has_wasmtime=false
command -v "$bun_cmd" &>/dev/null && has_bun=true
command -v "$deno_cmd" &>/dev/null && has_deno=true
command -v "$qjs_cmd" &>/dev/null && has_qjs=true
command -v wasmtime &>/dev/null && has_wasmtime=true

tish_dir="tests/core"
perf_dir="tests/core"
# Always use local target directory for consistent builds
target_dir="$(pwd)/target"
profile="debug"
summary_only=false
no_compile=false
limit=0
run_timeout=30

# Parse arguments
while [[ $# -gt 0 ]]; do
  case "$1" in
    --release) profile="release"; shift ;;
    --summary-only) summary_only=true; shift ;;
    --no-compile) no_compile=true; shift ;;
    --limit) limit="$2"; shift 2 ;;
    --timeout) run_timeout="$2"; shift 2 ;;
    *) shift ;;
  esac
done

# Timeout wrapper (avoids hanging on slow VM runs, e.g. array_methods_perf)
run_with_timeout() {
  if [[ $run_timeout -gt 0 ]]; then
    if command -v timeout &>/dev/null; then
      timeout "$run_timeout" "$@" 2>/dev/null || true
    elif command -v perl &>/dev/null; then
      perl -e 'alarm shift; exec @ARGV' "$run_timeout" "$@" 2>/dev/null || true
    else
      "$@" 2>/dev/null || true
    fi
  else
    "$@" 2>/dev/null || true
  fi
}

tish_bin="$target_dir/$profile/tish"
rel_flag=""
[[ "$profile" == "release" ]] && rel_flag="--release"
# Always build tish so we use latest codegen (cargo skips if unchanged)
echo "Building tish ($profile)..."
cargo build -p tish $rel_flag --target-dir "$target_dir" -q 2>/dev/null || true
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
declare -a summary_tish_cranelift=()
declare -a summary_tish_wasi=()
declare -a summary_node=()
declare -a summary_bun=()
declare -a summary_deno=()
declare -a summary_qjs=()
declare -a summary_ratio=()

# Track which tests compiled successfully (using files instead of associative array for bash 3 compat)

echo "=== Tish vs JavaScript Runtimes — Performance Comparison ==="
echo "Profile: $profile"
[[ $run_timeout -gt 0 ]] && echo "Tish run timeout: ${run_timeout}s (use --timeout 0 to disable)"
echo ""
echo "Detected runtimes:"
echo "  Tish: runtime (interpreter), rust native, cranelift native, wasi (wasmtime)"
echo "  Node.js: $("$node_cmd" --version 2>/dev/null || echo 'not found')"
$has_bun && echo "  Bun: $("$bun_cmd" --version 2>/dev/null || echo 'unknown')"
$has_deno && echo "  Deno: $("$deno_cmd" --version 2>/dev/null | head -1 || echo 'unknown')"
$has_qjs && echo "  QuickJS: detected"
$has_wasmtime && echo "  Wasmtime: $(wasmtime --version 2>/dev/null | head -1 || echo 'detected')" || echo "  Wasmtime: not found (skip WASI)"
echo ""

# Phase 1: Pre-compile all Tish binaries (rust native, cranelift, wasi)
if $no_compile; then
  cached_native=$(find "$compile_dir" -name '*_native' -type f \( -perm +111 -o -perm +1 \) 2>/dev/null | wc -l | tr -d ' ')
  cached_cranelift=$(find "$compile_dir" -name '*_cranelift' -type f \( -perm +111 -o -perm +1 \) 2>/dev/null | wc -l | tr -d ' ')
  cached_wasi=$(find "$compile_dir" -name '*.wasm' -type f 2>/dev/null | wc -l | tr -d ' ')
  echo "Using cached binaries from: $compile_dir"
  echo "Cached: rust=$cached_native cranelift=$cached_cranelift wasi=$cached_wasi"
  echo ""
else
  echo "Compiling Tish binaries (rust native, cranelift, wasi)..."
  rust_ok=0 rust_skip=0
  cranelift_ok=0 cranelift_skip=0
  wasi_ok=0 wasi_skip=0
  count=0
  for f in "$perf_dir"/*.js; do
    [[ -f "$f" ]] || continue
    [[ $limit -gt 0 && $count -ge $limit ]] && break
    base=$(basename "$f" .js)
    tish_file="$tish_dir/$base.tish"
    [[ -f "$tish_file" ]] || continue
    count=$((count + 1))

    echo -n "  $base: "
    # Rust native (default backend)
    if $tish_bin compile "$tish_file" -o "$compile_dir/${base}_native" --native-backend rust >/dev/null 2>&1; then
      echo -n "rust "
      rust_ok=$((rust_ok + 1))
    else
      echo -n "rust-skip "
      rust_skip=$((rust_skip + 1))
    fi
    # Cranelift (pure Tish, no native imports)
    if $tish_bin compile "$tish_file" -o "$compile_dir/${base}_cranelift" --native-backend cranelift >/dev/null 2>&1; then
      echo -n "cranelift "
      cranelift_ok=$((cranelift_ok + 1))
    else
      echo -n "cranelift-skip "
      cranelift_skip=$((cranelift_skip + 1))
    fi
    # WASI (for wasmtime)
    if $has_wasmtime; then
      if $tish_bin compile "$tish_file" -o "$compile_dir/${base}_wasi" --target wasi >/dev/null 2>&1; then
        echo -n "wasi"
        wasi_ok=$((wasi_ok + 1))
      else
        echo -n "wasi-skip"
        wasi_skip=$((wasi_skip + 1))
      fi
    else
      echo -n "wasi-skip (no wasmtime)"
    fi
    echo ""
  done
  echo "Compiled: rust=$rust_ok/$((rust_ok+rust_skip)) cranelift=$cranelift_ok/$((cranelift_ok+cranelift_skip)) wasi=$wasi_ok/$((wasi_ok+wasi_skip))"
  echo "Cache location: $compile_dir"
  echo ""
fi

# Phase 2: Run timing tests
echo "Running performance tests..."
count=0
for f in "$perf_dir"/*.js; do
  [[ -f "$f" ]] || continue
  [[ $limit -gt 0 && $count -ge $limit ]] && break
  base=$(basename "$f" .js)
  tish_file="$tish_dir/$base.tish"
  [[ -f "$tish_file" ]] || continue
  count=$((count + 1))

  native_bin="$compile_dir/${base}_native"
  cranelift_bin="$compile_dir/${base}_cranelift"
  wasi_bin="$compile_dir/${base}_wasi.wasm"

  if ! $summary_only; then
    echo "─────────────────────────────────────────"
    echo "▶ $base"
    echo "─────────────────────────────────────────"

    # Output (use timeout to avoid hangs on intensive tests)
    echo "Tish (run):"
    run_with_timeout $tish_bin run "$tish_file" 2>&1 || true
    echo ""

    echo "Tish (rust):"
    if [[ -x "$native_bin" ]]; then
      run_with_timeout "$native_bin" 2>&1 || true
    else
      echo "(not built)"
    fi
    echo ""

    echo "Tish (cranelift):"
    if [[ -x "$cranelift_bin" ]]; then
      run_with_timeout "$cranelift_bin" 2>&1 || true
    else
      echo "(not built)"
    fi
    echo ""

    echo "Tish (wasi):"
    if $has_wasmtime && [[ -f "$wasi_bin" ]]; then
      run_with_timeout wasmtime "$wasi_bin" 2>&1 || true
    else
      echo "(not built or wasmtime not found)"
    fi
    echo ""

    echo "Node.js:"
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

    if $has_qjs; then
      echo "QuickJS:"
      "$qjs_cmd" "$f" 2>&1 || true
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
  tish_cranelift_times=()
  tish_wasi_times=()
  node_times=()
  bun_times=()
  deno_times=()
  qjs_times=()

  # Warmup runs (discard - warms disk cache and JIT; use timeout to avoid hangs)
  run_with_timeout $tish_bin run "$tish_file" >/dev/null 2>&1 || true
  [[ -x "$native_bin" ]] && "$native_bin" >/dev/null 2>&1 || true
  [[ -x "$cranelift_bin" ]] && "$cranelift_bin" >/dev/null 2>&1 || true
  $has_wasmtime && [[ -f "$wasi_bin" ]] && wasmtime "$wasi_bin" >/dev/null 2>&1 || true
  "$node_cmd" "$f" >/dev/null 2>&1 || true
  $has_bun && "$bun_cmd" "$f" >/dev/null 2>&1 || true
  $has_deno && "$deno_cmd" run --allow-all "$f" >/dev/null 2>&1 || true
  $has_qjs && "$qjs_cmd" "$f" >/dev/null 2>&1 || true

  # Tish interpreter (run) - use timeout to avoid hangs on intensive tests
  for _ in $(seq 1 "$n"); do
    t0=$(ms)
    run_with_timeout $tish_bin run "$tish_file" >/dev/null 2>&1 || true
    t1=$(ms)
    tish_run_times+=($((t1 - t0)))
  done

  # Tish rust native
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

  # Tish cranelift
  cranelift_ok=false
  if [[ -x "$cranelift_bin" ]]; then
    cranelift_ok=true
    for _ in $(seq 1 "$n"); do
      t0=$(ms)
      "$cranelift_bin" >/dev/null 2>&1 || true
      t1=$(ms)
      tish_cranelift_times+=($((t1 - t0)))
    done
  fi

  # Tish WASI (wasmtime)
  wasi_ok=false
  if $has_wasmtime && [[ -f "$wasi_bin" ]]; then
    wasi_ok=true
    for _ in $(seq 1 "$n"); do
      t0=$(ms)
      wasmtime "$wasi_bin" >/dev/null 2>&1 || true
      t1=$(ms)
      tish_wasi_times+=($((t1 - t0)))
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
  tish_cranelift_sum=0
  tish_wasi_sum=0
  node_sum=0
  bun_sum=0
  deno_sum=0
  qjs_sum=0

  for t in "${tish_run_times[@]}"; do tish_run_sum=$((tish_run_sum + t)); done
  for t in "${tish_native_times[@]}"; do tish_native_sum=$((tish_native_sum + t)); done
  for t in "${tish_cranelift_times[@]}"; do tish_cranelift_sum=$((tish_cranelift_sum + t)); done
  for t in "${tish_wasi_times[@]}"; do tish_wasi_sum=$((tish_wasi_sum + t)); done
  for t in "${node_times[@]}"; do node_sum=$((node_sum + t)); done

  tish_run_avg=$((tish_run_sum / n))
  tish_native_avg=0
  tish_cranelift_avg=0
  tish_wasi_avg=0
  $compile_ok && tish_native_avg=$((tish_native_sum / n))
  $cranelift_ok && tish_cranelift_avg=$((tish_cranelift_sum / n))
  $wasi_ok && tish_wasi_avg=$((tish_wasi_sum / n))
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
  summary_tish_cranelift+=("$tish_cranelift_avg")
  summary_tish_wasi+=("$tish_wasi_avg")
  summary_node+=("$node_avg")
  summary_bun+=("$bun_avg")
  summary_deno+=("$deno_avg")
  summary_qjs+=("$qjs_avg")
  summary_ratio+=("$ratio")

  if ! $summary_only; then
    echo "Time (${n} runs avg):"
    echo "  Tish (run):       ${tish_run_avg}ms"
    $compile_ok && echo "  Tish (rust):      ${tish_native_avg}ms"
    $cranelift_ok && echo "  Tish (cranelift): ${tish_cranelift_avg}ms"
    $wasi_ok && echo "  Tish (wasi):      ${tish_wasi_avg}ms"
    echo "  Node.js:          ${node_avg}ms"
    $has_bun && echo "  Bun:             ${bun_avg}ms"
    $has_deno && echo "  Deno:            ${deno_avg}ms"
    $has_qjs && echo "  QuickJS:         ${qjs_avg}ms"
    echo "  Tish(run)/Node ratio: ${ratio}%"
    echo ""
  else
    echo " done (run: ${tish_run_avg}ms rust: ${tish_native_avg}ms cranelift: ${tish_cranelift_avg}ms wasi: ${tish_wasi_avg}ms ratio: ${ratio}%)"
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
header="%-20s %8s %8s %8s %8s %8s"
header_args=("Test" "run" "rust" "cranelift" "wasi" "Node")
divider="────────────────────"
div_args=("──────" "──────" "──────" "──────" "──────")

if $has_bun; then
  header="$header %8s"
  header_args+=("Bun")
  div_args+=("──────")
fi
if $has_deno; then
  header="$header %8s"
  header_args+=("Deno")
  div_args+=("──────")
fi
if $has_qjs; then
  header="$header %8s"
  header_args+=("QuickJS")
  div_args+=("──────")
fi
header="$header %10s\n"
header_args+=("run/Node%")
div_args+=("──────────")

printf "$header" "${header_args[@]}"
printf "%-20s" "$divider"
for d in "${div_args[@]}"; do printf " %8s" "$d"; done
echo ""

# Print sorted results
total_tish_run=0
total_tish_native=0
total_tish_cranelift=0
total_tish_wasi=0
total_node=0
total_bun=0
total_deno=0
total_qjs=0

for entry in "${sorted_indices[@]}"; do
  idx="${entry#*:}"
  name="${summary_names[$idx]}"
  tish_run="${summary_tish_run[$idx]}"
  tish_native="${summary_tish_native[$idx]}"
  tish_cranelift="${summary_tish_cranelift[$idx]}"
  tish_wasi="${summary_tish_wasi[$idx]}"
  node="${summary_node[$idx]}"
  bun="${summary_bun[$idx]}"
  deno="${summary_deno[$idx]}"
  qjs="${summary_qjs[$idx]}"
  ratio="${summary_ratio[$idx]}"

  total_tish_run=$((total_tish_run + tish_run))
  total_tish_native=$((total_tish_native + tish_native))
  total_tish_cranelift=$((total_tish_cranelift + tish_cranelift))
  total_tish_wasi=$((total_tish_wasi + tish_wasi))
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

  # Display: show "-" if not available
  native_display="$tish_native"
  [[ $tish_native -eq 0 ]] && native_display="-"
  cranelift_display="$tish_cranelift"
  [[ $tish_cranelift -eq 0 ]] && cranelift_display="-"
  wasi_display="$tish_wasi"
  [[ $tish_wasi -eq 0 ]] && wasi_display="-"

  # Build row dynamically
  row="%-20s %8d %8s %8s %8s %8d"
  row_args=("$name" "$tish_run" "$native_display" "$cranelift_display" "$wasi_display" "$node")

  if $has_bun; then
    row="$row %8d"
    row_args+=("$bun")
  fi
  if $has_deno; then
    row="$row %8d"
    row_args+=("$deno")
  fi
  if $has_qjs; then
    row="$row %8d"
    row_args+=("$qjs")
  fi
  row="$row %9d%%"
  row_args+=("$ratio")

  printf "${color}${row}${reset}\n" "${row_args[@]}"
done

# Print totals
echo ""
printf "%-20s" "$divider"
for d in "${div_args[@]}"; do printf " %8s" "$d"; done
echo ""

if [[ $total_node -gt 0 ]]; then
  total_ratio=$((total_tish_run * 100 / total_node))
else
  total_ratio=0
fi

native_total_display="$total_tish_native"
[[ $total_tish_native -eq 0 ]] && native_total_display="-"
cranelift_total_display="$total_tish_cranelift"
[[ $total_tish_cranelift -eq 0 ]] && cranelift_total_display="-"
wasi_total_display="$total_tish_wasi"
[[ $total_tish_wasi -eq 0 ]] && wasi_total_display="-"

row="%-20s %8d %8s %8s %8s %8d"
row_args=("TOTAL" "$total_tish_run" "$native_total_display" "$cranelift_total_display" "$wasi_total_display" "$total_node")

if $has_bun; then
  row="$row %8d"
  row_args+=("$total_bun")
fi
if $has_deno; then
  row="$row %8d"
  row_args+=("$total_deno")
fi
if $has_qjs; then
  row="$row %8d"
  row_args+=("$total_qjs")
fi
row="$row %9d%%"
row_args+=("$total_ratio")

printf "$row\n" "${row_args[@]}"

echo ""
echo "Legend: Green = <150% | Yellow = 200-500% | Red = >500%"
echo "        run=interpreter | rust=native(rust) | cranelift=native(cranelift) | wasi=wasmtime"
echo ""
echo "─────────────────────────────────────────"
echo "Done."
