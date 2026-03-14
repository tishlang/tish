#!/bin/bash
# Run Tish and JS equivalents, show output and compare execution time.
# Usage: ./scripts/run_performance_manual.sh [--release] [--summary-only] [--no-compile] [--limit N] [--filter NAME] [--runtimes R,...]
#   --release       use release build (recommended for fair Tish vs JS timing)
#   --summary-only  skip individual test output, show only summary
#   --no-compile    skip compilation, use cached binaries from previous runs
#   --limit N       run only first N tests (default: all)
#   --filter NAME   run only tests whose dir/base contains NAME (e.g. array_stress, modules, modules/promise)
#   --timeout SEC   timeout per tish run in seconds (default: 30, 0=no limit)
#   --runtimes R,...  comma-separated list: vm,interp,rust,cranelift,llvm,wasi,node,bun,deno,qjs (default: all)
#   --verbose       show stderr (crash logs, runtime errors) instead of suppressing them

set -e
cd "$(dirname "$0")/.."

# Check if a runtime is in the selected list (empty = all)
# "run" is shorthand for both vm and interp
want_runtime() {
  local r="$1"
  if [[ -z "$runtimes_filter" ]]; then
    return 0
  fi
  [[ ",${runtimes_filter}," == *",${r},"* ]] && return 0
  [[ "$r" == "vm" && ",${runtimes_filter}," == *",run,"* ]] && return 0
  [[ "$r" == "interp" && ",${runtimes_filter}," == *",run,"* ]] && return 0
  return 1
}

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

# Directories containing .js and matching .tish files
perf_dirs=("tests/core" "tests/modules")
# Always use local target directory for consistent builds
target_dir="$(pwd)/target"
profile="debug"
summary_only=false
verbose=false
no_compile=false
limit=0
run_timeout=30
runtimes_filter=""
filter_name=""
# Parse arguments
while [[ $# -gt 0 ]]; do
  case "$1" in
    --release) profile="release"; shift ;;
    --summary-only) summary_only=true; shift ;;
    --no-compile) no_compile=true; shift ;;
    --limit) limit="$2"; shift 2 ;;
    --filter) filter_name="$2"; shift 2 ;;
    --timeout) run_timeout="$2"; shift 2 ;;
    --runtimes) runtimes_filter="$2"; shift 2 ;;
    --verbose|-v) verbose=true; shift ;;
    *) shift ;;
  esac
done

# Timeout wrapper: stop process after run_timeout seconds. Uses timeout/gtimeout, perl, or bash.
# When verbose=true, stderr is shown; otherwise suppressed.
run_with_timeout() {
  if [[ $run_timeout -le 0 ]]; then
    if $verbose; then "$@" || true; else "$@" 2>/dev/null || true; fi
    return
  fi
  if command -v timeout &>/dev/null; then
    if $verbose; then timeout "$run_timeout" "$@" || true; else timeout "$run_timeout" "$@" 2>/dev/null || true; fi
    return
  fi
  if command -v gtimeout &>/dev/null; then
    if $verbose; then gtimeout "$run_timeout" "$@" || true; else gtimeout "$run_timeout" "$@" 2>/dev/null || true; fi
    return
  fi
  if command -v perl &>/dev/null; then
    # Fork; child uses setsid+exec so we can kill process group (handles cargo->tish)
    if $verbose; then
      perl -e '
        my $t=shift;
        my $pid=fork;
        die "fork: $!" if !defined $pid;
        if ($pid==0) { setpgrp(0,0) if defined &setpgrp; exec(@ARGV) or exit 127; }
        $SIG{ALRM}=sub{ kill 9,-$pid; kill 9,$pid; waitpid $pid,0; exit 124 };
        alarm $t; waitpid $pid,0; alarm 0; exit($?>>8)
      ' "$run_timeout" "$@" || true
    else
      perl -e '
        my $t=shift;
        my $pid=fork;
        die "fork: $!" if !defined $pid;
        if ($pid==0) { setpgrp(0,0) if defined &setpgrp; exec(@ARGV) or exit 127; }
        $SIG{ALRM}=sub{ kill 9,-$pid; kill 9,$pid; waitpid $pid,0; exit 124 };
        alarm $t; waitpid $pid,0; alarm 0; exit($?>>8)
      ' "$run_timeout" "$@" 2>/dev/null || true
    fi
    return
  fi
  ( set +e; set -m; "$@" 2>/dev/null & pid=$!; ( sleep $run_timeout; kill -TERM -$pid 2>/dev/null; sleep 2; kill -KILL -$pid 2>/dev/null ) & k=$!; wait $pid 2>/dev/null; kill $k 2>/dev/null; wait $k 2>/dev/null ) || true
}

tish_bin="$target_dir/$profile/tish"
rel_flag=""
[[ "$profile" == "release" ]] && rel_flag="--release"
# Always build tish so we use latest codegen (cargo skips if unchanged)
echo "Building tish ($profile)..."
cargo build -p tish $rel_flag --features full --target-dir "$target_dir" -q 2>/dev/null || true
if [[ ! -x "$tish_bin" ]]; then
  tish_bin="cargo run -p tish $rel_flag --features full --target-dir $target_dir -q --"
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
declare -a summary_tish_vm=()
declare -a summary_tish_interp=()
declare -a summary_tish_native=()
declare -a summary_tish_cranelift=()
declare -a summary_tish_llvm=()
declare -a summary_tish_wasi=()
declare -a summary_node=()
declare -a summary_bun=()
declare -a summary_deno=()
declare -a summary_qjs=()
declare -a summary_ratio=()

# Track which tests compiled successfully (using files instead of associative array for bash 3 compat)

echo "=== Tish vs JavaScript Runtimes — Performance Comparison ==="
echo "Profile: $profile"
[[ -n "$runtimes_filter" ]] && echo "Runtimes: $runtimes_filter (use --runtimes vm,interp,rust,cranelift,llvm,wasi,node,bun,deno,qjs)"
$verbose && echo "Verbose: stderr (crash logs) will be shown"
[[ $run_timeout -gt 0 ]] && echo "Tish run timeout: ${run_timeout}s (use --timeout 0 to disable)"
echo ""
echo "Runtimes to test:"
want_runtime vm && echo "  vm (tish run --backend vm)"
want_runtime interp && echo "  interp (tish run --backend interp)"
want_runtime rust && echo "  rust (tish native)"
want_runtime cranelift && echo "  cranelift (tish JIT)"
want_runtime llvm && echo "  llvm (tish native via clang)"
want_runtime wasi && echo "  wasi (wasmtime)"
want_runtime node && echo "  node"
want_runtime bun && $has_bun && echo "  bun"
want_runtime deno && $has_deno && echo "  deno"
want_runtime qjs && $has_qjs && echo "  qjs"
[[ -n "$filter_name" ]] && echo "Filter: $filter_name"
[[ $limit -gt 0 ]] && echo "Limit: $limit"
echo ""

# Phase 1: Pre-compile all Tish binaries (rust native, cranelift, wasi)
if $no_compile; then
  cached_native=$(find "$compile_dir" -name '*_native' -type f \( -perm +111 -o -perm +1 \) 2>/dev/null | wc -l | tr -d ' ')
  cached_cranelift=$(find "$compile_dir" -name '*_cranelift' -type f \( -perm +111 -o -perm +1 \) 2>/dev/null | wc -l | tr -d ' ')
  cached_llvm=$(find "$compile_dir" -name '*_llvm' -type f \( -perm +111 -o -perm +1 \) 2>/dev/null | wc -l | tr -d ' ')
  cached_wasi=$(find "$compile_dir" -name '*.wasm' -type f 2>/dev/null | wc -l | tr -d ' ')
  echo "Using cached binaries from: $compile_dir"
  echo "Cached: rust=$cached_native cranelift=$cached_cranelift llvm=$cached_llvm wasi=$cached_wasi"
  echo ""
else
  echo "Compiling Tish binaries (rust native, cranelift, llvm, wasi)..."
  rust_ok=0 rust_skip=0
  cranelift_ok=0 cranelift_skip=0
  llvm_ok=0 llvm_skip=0
  wasi_ok=0 wasi_skip=0
  count=0
  for perf_dir in "${perf_dirs[@]}"; do
    [[ -d "$perf_dir" ]] || continue
    for f in "$perf_dir"/*.js; do
      [[ -f "$f" ]] || continue
      base=$(basename "$f" .js)
      test_id="${perf_dir#tests/}/$base"
      [[ "$base" == "recursion_stress" ]] && continue
      [[ -n "$filter_name" && "$test_id" != *"$filter_name"* ]] && continue
      [[ $limit -gt 0 && $count -ge $limit ]] && break
      tish_file="$perf_dir/$base.tish"
      [[ -f "$tish_file" ]] || continue
      count=$((count + 1))
      cache_key="${test_id//\//_}"

    echo -n "  $test_id: "
    # Rust native (default backend)
    if want_runtime rust; then
      if $tish_bin compile "$tish_file" -o "$compile_dir/${cache_key}_native" --native-backend rust >/dev/null 2>&1; then
        echo -n "rust "
        rust_ok=$((rust_ok + 1))
      else
        echo -n "rust-skip "
        rust_skip=$((rust_skip + 1))
      fi
    fi
    # Cranelift (pure Tish, no native imports)
    if want_runtime cranelift; then
      if $tish_bin compile "$tish_file" -o "$compile_dir/${cache_key}_cranelift" --native-backend cranelift >/dev/null 2>&1; then
        echo -n "cranelift "
        cranelift_ok=$((cranelift_ok + 1))
      else
        echo -n "cranelift-skip "
        cranelift_skip=$((cranelift_skip + 1))
      fi
    fi
    # LLVM (pure Tish, no native imports; uses clang)
    if want_runtime llvm; then
      if $tish_bin compile "$tish_file" -o "$compile_dir/${cache_key}_llvm" --native-backend llvm >/dev/null 2>&1; then
        echo -n "llvm "
        llvm_ok=$((llvm_ok + 1))
      else
        echo -n "llvm-skip "
        llvm_skip=$((llvm_skip + 1))
      fi
    fi
    # WASI (for wasmtime)
    if want_runtime wasi; then
      if $has_wasmtime; then
        if $tish_bin compile "$tish_file" -o "$compile_dir/${cache_key}_wasi" --target wasi >/dev/null 2>&1; then
          echo -n "wasi"
          wasi_ok=$((wasi_ok + 1))
        else
          echo -n "wasi-skip"
          wasi_skip=$((wasi_skip + 1))
        fi
      else
        echo -n "wasi-skip (no wasmtime)"
      fi
    fi
    echo ""
    done
  done
  echo "Compiled: rust=$rust_ok/$((rust_ok+rust_skip)) cranelift=$cranelift_ok/$((cranelift_ok+cranelift_skip)) llvm=$llvm_ok/$((llvm_ok+llvm_skip)) wasi=$wasi_ok/$((wasi_ok+wasi_skip))"
  echo "Cache location: $compile_dir"
  echo ""
fi

# Phase 2: Run timing tests
echo "Running performance tests..."
count=0
for perf_dir in "${perf_dirs[@]}"; do
  [[ -d "$perf_dir" ]] || continue
  for f in "$perf_dir"/*.js; do
    [[ -f "$f" ]] || continue
    base=$(basename "$f" .js)
    test_id="${perf_dir#tests/}/$base"
    [[ "$base" == "recursion_stress" ]] && continue
    [[ -n "$filter_name" && "$test_id" != *"$filter_name"* ]] && continue
    [[ $limit -gt 0 && $count -ge $limit ]] && break
    tish_file="$perf_dir/$base.tish"
    [[ -f "$tish_file" ]] || continue
    count=$((count + 1))
    cache_key="${test_id//\//_}"

  native_bin="$compile_dir/${cache_key}_native"
  cranelift_bin="$compile_dir/${cache_key}_cranelift"
  llvm_bin="$compile_dir/${cache_key}_llvm"
  wasi_bin="$compile_dir/${cache_key}_wasi.wasm"

  if ! $summary_only; then
    echo "─────────────────────────────────────────"
    echo "▶ $test_id"
    echo "─────────────────────────────────────────"

    # Output (use timeout to avoid hangs on intensive tests)
    if want_runtime run; then
      echo "Tish (run):"
      run_with_timeout $tish_bin run "$tish_file" 2>&1 || true
      echo ""
    fi

    if want_runtime rust; then
      echo "Tish (rust):"
      if [[ -x "$native_bin" ]]; then
        run_with_timeout "$native_bin" 2>&1 || true
      else
        echo "(not built)"
      fi
      echo ""
    fi

    if want_runtime cranelift; then
      echo "Tish (cranelift):"
      if [[ -x "$cranelift_bin" ]]; then
        run_with_timeout "$cranelift_bin" 2>&1 || true
      else
        echo "(not built)"
      fi
      echo ""
    fi

    if want_runtime llvm; then
      echo "Tish (llvm):"
      if [[ -x "$llvm_bin" ]]; then
        run_with_timeout "$llvm_bin" 2>&1 || true
      else
        echo "(not built)"
      fi
      echo ""
    fi

    if want_runtime wasi; then
      echo "Tish (wasi):"
      if $has_wasmtime && [[ -f "$wasi_bin" ]]; then
        run_with_timeout wasmtime "$wasi_bin" 2>&1 || true
      else
        echo "(not built or wasmtime not found)"
      fi
      echo ""
    fi

    if want_runtime node; then
      echo "Node.js:"
      "$node_cmd" "$f" 2>&1 || true
      echo ""
    fi

    if want_runtime bun && $has_bun; then
      echo "Bun:"
      "$bun_cmd" "$f" 2>&1 || true
      echo ""
    fi

    if want_runtime deno && $has_deno; then
      echo "Deno:"
      "$deno_cmd" run --allow-all "$f" 2>&1 || true
      echo ""
    fi

    if want_runtime qjs && $has_qjs; then
      echo "QuickJS:"
      "$qjs_cmd" "$f" 2>&1 || true
      echo ""
    fi
  else
    echo -n "Running $test_id..."
  fi

  # Timing (multiple runs to reduce noise; report average)
  #n=50
  n=5
  tish_vm_times=()
  tish_interp_times=()
  tish_native_times=()
  tish_cranelift_times=()
  tish_llvm_times=()
  tish_wasi_times=()
  node_times=()
  bun_times=()
  deno_times=()
  qjs_times=()

  # Warmup runs (discard - warms disk cache and JIT; use timeout to avoid hangs)
  want_runtime vm && run_with_timeout $tish_bin run "$tish_file" --backend vm >/dev/null 2>&1 || true
  want_runtime interp && run_with_timeout $tish_bin run "$tish_file" --backend interp >/dev/null 2>&1 || true
  want_runtime rust && [[ -x "$native_bin" ]] && run_with_timeout "$native_bin" >/dev/null 2>&1 || true
  want_runtime cranelift && [[ -x "$cranelift_bin" ]] && run_with_timeout "$cranelift_bin" >/dev/null 2>&1 || true
  want_runtime llvm && [[ -x "$llvm_bin" ]] && run_with_timeout "$llvm_bin" >/dev/null 2>&1 || true
  want_runtime wasi && $has_wasmtime && [[ -f "$wasi_bin" ]] && run_with_timeout wasmtime "$wasi_bin" >/dev/null 2>&1 || true
  want_runtime node && "$node_cmd" "$f" >/dev/null 2>&1 || true
  want_runtime bun && $has_bun && "$bun_cmd" "$f" >/dev/null 2>&1 || true
  want_runtime deno && $has_deno && "$deno_cmd" run --allow-all "$f" >/dev/null 2>&1 || true
  want_runtime qjs && $has_qjs && "$qjs_cmd" "$f" >/dev/null 2>&1 || true

  # Tish VM (run --backend vm)
  if want_runtime vm; then
    for _ in $(seq 1 "$n"); do
      t0=$(ms)
      run_with_timeout $tish_bin run "$tish_file" --backend vm >/dev/null 2>&1 || true
      t1=$(ms)
      tish_vm_times+=($((t1 - t0)))
    done
  fi

  # Tish interpreter (run --backend interp)
  if want_runtime interp; then
    for _ in $(seq 1 "$n"); do
      t0=$(ms)
      run_with_timeout $tish_bin run "$tish_file" --backend interp >/dev/null 2>&1 || true
      t1=$(ms)
      tish_interp_times+=($((t1 - t0)))
    done
  fi

  # Tish rust native
  compile_ok=false
  if want_runtime rust && [[ -x "$native_bin" ]]; then
    compile_ok=true
    for _ in $(seq 1 "$n"); do
      t0=$(ms)
      run_with_timeout "$native_bin" >/dev/null 2>&1 || true
      t1=$(ms)
      tish_native_times+=($((t1 - t0)))
    done
  fi

  # Tish cranelift
  cranelift_ok=false
  if want_runtime cranelift && [[ -x "$cranelift_bin" ]]; then
    cranelift_ok=true
    for _ in $(seq 1 "$n"); do
      t0=$(ms)
      run_with_timeout "$cranelift_bin" >/dev/null 2>&1 || true
      t1=$(ms)
      tish_cranelift_times+=($((t1 - t0)))
    done
  fi

  # Tish llvm
  llvm_ok=false
  if want_runtime llvm && [[ -x "$llvm_bin" ]]; then
    llvm_ok=true
    for _ in $(seq 1 "$n"); do
      t0=$(ms)
      run_with_timeout "$llvm_bin" >/dev/null 2>&1 || true
      t1=$(ms)
      tish_llvm_times+=($((t1 - t0)))
    done
  fi

  # Tish WASI (wasmtime)
  wasi_ok=false
  if want_runtime wasi && $has_wasmtime && [[ -f "$wasi_bin" ]]; then
    wasi_ok=true
    for _ in $(seq 1 "$n"); do
      t0=$(ms)
      run_with_timeout wasmtime "$wasi_bin" >/dev/null 2>&1 || true
      t1=$(ms)
      tish_wasi_times+=($((t1 - t0)))
    done
  fi

  # Node.js
  if want_runtime node; then
    for _ in $(seq 1 "$n"); do
      t0=$(ms)
      "$node_cmd" "$f" >/dev/null 2>&1 || true
      t1=$(ms)
      node_times+=($((t1 - t0)))
    done
  fi

  # Bun
  if want_runtime bun && $has_bun; then
    for _ in $(seq 1 "$n"); do
      t0=$(ms)
      "$bun_cmd" "$f" >/dev/null 2>&1 || true
      t1=$(ms)
      bun_times+=($((t1 - t0)))
    done
  fi

  # Deno
  if want_runtime deno && $has_deno; then
    for _ in $(seq 1 "$n"); do
      t0=$(ms)
      "$deno_cmd" run --allow-all "$f" >/dev/null 2>&1 || true
      t1=$(ms)
      deno_times+=($((t1 - t0)))
    done
  fi

  # QuickJS
  if want_runtime qjs && $has_qjs; then
    for _ in $(seq 1 "$n"); do
      t0=$(ms)
      "$qjs_cmd" "$f" >/dev/null 2>&1 || true
      t1=$(ms)
      qjs_times+=($((t1 - t0)))
    done
  fi

  # Calculate averages
  tish_vm_sum=0
  tish_interp_sum=0
  tish_native_sum=0
  tish_cranelift_sum=0
  tish_llvm_sum=0
  tish_wasi_sum=0
  node_sum=0
  bun_sum=0
  deno_sum=0
  qjs_sum=0

  for t in "${tish_vm_times[@]}"; do tish_vm_sum=$((tish_vm_sum + t)); done
  for t in "${tish_interp_times[@]}"; do tish_interp_sum=$((tish_interp_sum + t)); done
  for t in "${tish_native_times[@]}"; do tish_native_sum=$((tish_native_sum + t)); done
  for t in "${tish_cranelift_times[@]}"; do tish_cranelift_sum=$((tish_cranelift_sum + t)); done
  for t in "${tish_llvm_times[@]}"; do tish_llvm_sum=$((tish_llvm_sum + t)); done
  for t in "${tish_wasi_times[@]}"; do tish_wasi_sum=$((tish_wasi_sum + t)); done
  for t in "${node_times[@]}"; do node_sum=$((node_sum + t)); done

  tish_vm_avg=$((tish_vm_sum / n))
  tish_interp_avg=$((tish_interp_sum / n))
  tish_native_avg=0
  tish_cranelift_avg=0
  tish_llvm_avg=0
  tish_wasi_avg=0
  $compile_ok && tish_native_avg=$((tish_native_sum / n))
  $cranelift_ok && tish_cranelift_avg=$((tish_cranelift_sum / n))
  $llvm_ok && tish_llvm_avg=$((tish_llvm_sum / n))
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
    ratio=$((tish_vm_avg * 100 / node_avg))
  fi

  # Store results for summary
  summary_names+=("$test_id")
  summary_tish_vm+=("$tish_vm_avg")
  summary_tish_interp+=("$tish_interp_avg")
  summary_tish_native+=("$tish_native_avg")
  summary_tish_cranelift+=("$tish_cranelift_avg")
  summary_tish_llvm+=("$tish_llvm_avg")
  summary_tish_wasi+=("$tish_wasi_avg")
  summary_node+=("$node_avg")
  summary_bun+=("$bun_avg")
  summary_deno+=("$deno_avg")
  summary_qjs+=("$qjs_avg")
  summary_ratio+=("$ratio")

  if ! $summary_only; then
    echo "Time (${n} runs avg):"
    want_runtime vm && echo "  Tish (vm):        ${tish_vm_avg}ms"
    want_runtime interp && echo "  Tish (interp):    ${tish_interp_avg}ms"
    want_runtime rust && $compile_ok && echo "  Tish (rust):      ${tish_native_avg}ms"
    want_runtime cranelift && $cranelift_ok && echo "  Tish (cranelift): ${tish_cranelift_avg}ms"
    want_runtime llvm && $llvm_ok && echo "  Tish (llvm):      ${tish_llvm_avg}ms"
    want_runtime wasi && $wasi_ok && echo "  Tish (wasi):      ${tish_wasi_avg}ms"
    want_runtime node && echo "  Node.js:          ${node_avg}ms"
    want_runtime bun && $has_bun && echo "  Bun:             ${bun_avg}ms"
    want_runtime deno && $has_deno && echo "  Deno:            ${deno_avg}ms"
    want_runtime qjs && $has_qjs && echo "  QuickJS:         ${qjs_avg}ms"
    want_runtime node && want_runtime vm && echo "  Tish(vm)/Node ratio: ${ratio}%"
    echo ""
  else
    echo " done (vm: ${tish_vm_avg}ms interp: ${tish_interp_avg}ms rust: ${tish_native_avg}ms cranelift: ${tish_cranelift_avg}ms llvm: ${tish_llvm_avg}ms wasi: ${tish_wasi_avg}ms ratio: ${ratio}%)"
  fi
  done
done

# Print summary sorted by Tish(vm)/Node ratio (highest/slowest first)
echo ""
echo "════════════════════════════════════════════════════════════════════════════════════════════════════════════════"
echo "                                           PERFORMANCE SUMMARY"
echo "                                    (sorted by Tish(vm)/Node ratio, slowest first)"
echo "════════════════════════════════════════════════════════════════════════════════════════════════════════════════"
echo ""

# Create sortable data and sort by ratio (descending)
sorted_indices=()
for i in "${!summary_ratio[@]}"; do
  sorted_indices+=("${summary_ratio[$i]}:$i")
done
IFS=$'\n' sorted_indices=($(sort -t: -k1 -nr <<<"${sorted_indices[*]}")); unset IFS

# Build header dynamically based on selected runtimes
header="%-20s"
header_args=("Test")
divider="────────────────────"
div_args=()
want_runtime vm && { header="$header %8s"; header_args+=("vm"); div_args+=("──────"); }
want_runtime interp && { header="$header %8s"; header_args+=("interp"); div_args+=("──────"); }
want_runtime rust && { header="$header %8s"; header_args+=("rust"); div_args+=("──────"); }
want_runtime cranelift && { header="$header %8s"; header_args+=("cranelift"); div_args+=("──────"); }
want_runtime llvm && { header="$header %8s"; header_args+=("llvm"); div_args+=("──────"); }
want_runtime wasi && { header="$header %8s"; header_args+=("wasi"); div_args+=("──────"); }
want_runtime node && { header="$header %8s"; header_args+=("Node"); div_args+=("──────"); }
want_runtime bun && $has_bun && { header="$header %8s"; header_args+=("Bun"); div_args+=("──────"); }
want_runtime deno && $has_deno && { header="$header %8s"; header_args+=("Deno"); div_args+=("──────"); }
want_runtime qjs && $has_qjs && { header="$header %8s"; header_args+=("QuickJS"); div_args+=("──────"); }
want_runtime vm && want_runtime node && { header="$header %10s"; header_args+=("vm/Node%"); div_args+=("──────────"); }
header="$header\n"

printf "$header" "${header_args[@]}"
printf "%-20s" "$divider"
for d in "${div_args[@]}"; do printf " %8s" "$d"; done
printf "\n"

# Print sorted results
total_tish_vm=0
total_tish_interp=0
total_tish_native=0
total_tish_cranelift=0
total_tish_llvm=0
total_tish_wasi=0
total_node=0
total_bun=0
total_deno=0
total_qjs=0

for entry in "${sorted_indices[@]}"; do
  idx="${entry#*:}"
  name="${summary_names[$idx]}"
  tish_vm="${summary_tish_vm[$idx]}"
  tish_interp="${summary_tish_interp[$idx]}"
  tish_native="${summary_tish_native[$idx]}"
  tish_cranelift="${summary_tish_cranelift[$idx]}"
  tish_llvm="${summary_tish_llvm[$idx]}"
  tish_wasi="${summary_tish_wasi[$idx]}"
  node="${summary_node[$idx]}"
  bun="${summary_bun[$idx]}"
  deno="${summary_deno[$idx]}"
  qjs="${summary_qjs[$idx]}"
  ratio="${summary_ratio[$idx]}"

  total_tish_vm=$((total_tish_vm + tish_vm))
  total_tish_interp=$((total_tish_interp + tish_interp))
  total_tish_native=$((total_tish_native + tish_native))
  total_tish_cranelift=$((total_tish_cranelift + tish_cranelift))
  total_tish_llvm=$((total_tish_llvm + tish_llvm))
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
  llvm_display="$tish_llvm"
  [[ $tish_llvm -eq 0 ]] && llvm_display="-"
  wasi_display="$tish_wasi"
  [[ $tish_wasi -eq 0 ]] && wasi_display="-"

  # Build row dynamically (same order as header)
  row="%-20s"
  row_args=("$name")
  want_runtime vm && { row="$row %8d"; row_args+=("$tish_vm"); }
  want_runtime interp && { row="$row %8d"; row_args+=("$tish_interp"); }
  want_runtime rust && { row="$row %8s"; row_args+=("$native_display"); }
  want_runtime cranelift && { row="$row %8s"; row_args+=("$cranelift_display"); }
  want_runtime llvm && { row="$row %8s"; row_args+=("$llvm_display"); }
  want_runtime wasi && { row="$row %8s"; row_args+=("$wasi_display"); }
  want_runtime node && { row="$row %8d"; row_args+=("$node"); }
  want_runtime bun && $has_bun && { row="$row %8d"; row_args+=("$bun"); }
  want_runtime deno && $has_deno && { row="$row %8d"; row_args+=("$deno"); }
  want_runtime qjs && $has_qjs && { row="$row %8d"; row_args+=("$qjs"); }
  want_runtime vm && want_runtime node && { row="$row %9d%%"; row_args+=("$ratio"); }
  row="$row\n"

  printf "${color}${row}${reset}" "${row_args[@]}"
done

# Print totals
echo ""
printf "%-20s" "$divider"
for d in "${div_args[@]}"; do printf " %8s" "$d"; done
printf "\n"

if [[ $total_node -gt 0 ]]; then
  total_ratio=$((total_tish_vm * 100 / total_node))
else
  total_ratio=0
fi

native_total_display="$total_tish_native"
[[ $total_tish_native -eq 0 ]] && native_total_display="-"
cranelift_total_display="$total_tish_cranelift"
[[ $total_tish_cranelift -eq 0 ]] && cranelift_total_display="-"
llvm_total_display="$total_tish_llvm"
[[ $total_tish_llvm -eq 0 ]] && llvm_total_display="-"
wasi_total_display="$total_tish_wasi"
[[ $total_tish_wasi -eq 0 ]] && wasi_total_display="-"

row="%-20s"
row_args=("TOTAL")
want_runtime vm && { row="$row %8d"; row_args+=("$total_tish_vm"); }
want_runtime interp && { row="$row %8d"; row_args+=("$total_tish_interp"); }
want_runtime rust && { row="$row %8s"; row_args+=("$native_total_display"); }
want_runtime cranelift && { row="$row %8s"; row_args+=("$cranelift_total_display"); }
want_runtime llvm && { row="$row %8s"; row_args+=("$llvm_total_display"); }
want_runtime wasi && { row="$row %8s"; row_args+=("$wasi_total_display"); }
want_runtime node && { row="$row %8d"; row_args+=("$total_node"); }
want_runtime bun && $has_bun && { row="$row %8d"; row_args+=("$total_bun"); }
want_runtime deno && $has_deno && { row="$row %8d"; row_args+=("$total_deno"); }
want_runtime qjs && $has_qjs && { row="$row %8d"; row_args+=("$total_qjs"); }
want_runtime vm && want_runtime node && { row="$row %9d%%"; row_args+=("$total_ratio"); }
row="$row\n"

printf "$row" "${row_args[@]}"

echo ""
echo "Legend: Green = <150% | Yellow = 200-500% | Red = >500%"
echo "        vm=tish run --backend vm | interp=tish run --backend interp | rust=native(rust) | cranelift=native(cranelift) | llvm=native(llvm) | wasi=wasmtime"
echo ""
echo "─────────────────────────────────────────"
echo "Done."
