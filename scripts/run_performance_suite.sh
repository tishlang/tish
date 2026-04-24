#!/bin/bash
# Bundled perf suite: times tests/main.tish (+ tests/main.js for Node) with one native binary per backend.
# Regenerate sources: python3 scripts/generate_perf_ci_main.py
# Usage: ./scripts/run_performance_suite.sh [--release] [--summary-only] [--no-compile] [--timeout SEC] [--runtimes R,...] [--verbose]
#   --github-step-summary   append a markdown section to GITHUB_STEP_SUMMARY (CI)
#
# For per-file Tish vs JS comparison, use ./scripts/run_performance_manual.sh

set -euo pipefail
cd "$(dirname "$0")/.."

entry_tish="tests/main.tish"
entry_js="tests/main.js"
test_id="suite/main"

want_runtime() {
  local r="$1"
  if [[ -z "${runtimes_filter:-}" ]]; then
    return 0
  fi
  [[ ",${runtimes_filter}," == *",${r},"* ]] && return 0
  [[ "$r" == "vm" && ",${runtimes_filter}," == *",run,"* ]] && return 0
  [[ "$r" == "interp" && ",${runtimes_filter}," == *",run,"* ]] && return 0
  return 1
}

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

target_dir="$(pwd)/target"
profile="debug"
summary_only=false
verbose=false
no_compile=false
run_timeout=120
runtimes_filter=""
github_step_summary=false

while [[ $# -gt 0 ]]; do
  case "$1" in
    --release) profile="release"; shift ;;
    --summary-only) summary_only=true; shift ;;
    --no-compile) no_compile=true; shift ;;
    --timeout) run_timeout="$2"; shift 2 ;;
    --runtimes) runtimes_filter="$2"; shift 2 ;;
    --verbose|-v) verbose=true; shift ;;
    --github-step-summary) github_step_summary=true; shift ;;
    *) shift ;;
  esac
done

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

if [[ ! -f "$entry_tish" ]] || [[ ! -f "$entry_js" ]]; then
  echo "Missing $entry_tish or $entry_js — run: python3 scripts/generate_perf_ci_main.py"
  exit 1
fi

echo "Building tish ($profile)..."
cargo build -p tishlang$rel_flag --features full --target-dir "$target_dir" -q 2>/dev/null || true
if [[ ! -x "$tish_bin" ]]; then
  tish_bin="cargo run -p tishlang$rel_flag --features full --target-dir $target_dir -q --"
fi

cache_dir="$target_dir/perf-suite-cache-$profile"
if $no_compile; then
  compile_dir="$cache_dir"
  if [[ ! -d "$compile_dir" ]]; then
    echo "Error: No cached binaries at $compile_dir"
    exit 1
  fi
else
  compile_dir="$cache_dir"
  mkdir -p "$compile_dir"
fi

if command -v perl &>/dev/null; then
  ms() { perl -MTime::HiRes=time -e 'printf "%d\n", time*1000'; }
elif command -v python3 &>/dev/null; then
  ms() { python3 -c 'import time; print(int(time.time()*1000))'; }
else
  ms() { "$node_cmd" -e 'console.log(Date.now())'; }
fi

cache_key="ci_main_suite"
native_bin="$compile_dir/${cache_key}_native"
cranelift_bin="$compile_dir/${cache_key}_cranelift"
llvm_bin="$compile_dir/${cache_key}_llvm"
wasi_bin="$compile_dir/${cache_key}_wasi.wasm"
js_file="$entry_js"

echo "=== Tish bundled perf suite ($test_id) ==="
echo "Profile: $profile"
echo "Entry: $entry_tish"
[[ -n "${runtimes_filter:-}" ]] && echo "Runtimes: $runtimes_filter"
[[ $run_timeout -gt 0 ]] && echo "Timeout per run: ${run_timeout}s"
echo ""

if ! $no_compile; then
  echo "Compiling suite (rust / cranelift / llvm / wasi)..."
  echo -n "  $test_id: "
  if want_runtime rust; then
    if $tish_bin build "$entry_tish" -o "$native_bin" --native-backend rust >/dev/null 2>&1; then
      echo -n "rust "
    else
      echo -n "rust-fail "
    fi
  fi
  if want_runtime cranelift; then
    if $tish_bin build "$entry_tish" -o "$cranelift_bin" --native-backend cranelift >/dev/null 2>&1; then
      echo -n "cranelift "
    else
      echo -n "cranelift-fail "
    fi
  fi
  if want_runtime llvm; then
    if $tish_bin build "$entry_tish" -o "$llvm_bin" --native-backend llvm >/dev/null 2>&1; then
      echo -n "llvm "
    else
      echo -n "llvm-fail "
    fi
  fi
  if want_runtime wasi; then
    if $has_wasmtime; then
      if $tish_bin build "$entry_tish" -o "$compile_dir/${cache_key}_wasi" --target wasi >/dev/null 2>&1; then
        echo -n "wasi"
      else
        echo -n "wasi-fail"
      fi
    else
      echo -n "wasi-skip"
    fi
  fi
  echo ""
  echo ""
fi

if ! $summary_only; then
  echo "─────────────────────────────────────────"
  echo "▶ $test_id"
  echo "─────────────────────────────────────────"
  want_runtime run && { echo "Tish (run):"; run_with_timeout $tish_bin run "$entry_tish" 2>&1 || true; echo ""; }
  want_runtime rust && [[ -x "$native_bin" ]] && { echo "Tish (rust):"; run_with_timeout "$native_bin" 2>&1 || true; echo ""; }
  want_runtime cranelift && [[ -x "$cranelift_bin" ]] && { echo "Tish (cranelift):"; run_with_timeout "$cranelift_bin" 2>&1 || true; echo ""; }
  want_runtime llvm && [[ -x "$llvm_bin" ]] && { echo "Tish (llvm):"; run_with_timeout "$llvm_bin" 2>&1 || true; echo ""; }
  want_runtime wasi && $has_wasmtime && [[ -f "$wasi_bin" ]] && { echo "Tish (wasi):"; run_with_timeout wasmtime --dir /tmp "$wasi_bin" 2>&1 || true; echo ""; }
  want_runtime node && { echo "Node.js:"; "$node_cmd" "$js_file" 2>&1 || true; echo ""; }
fi

n=5
tish_vm_times=()
tish_interp_times=()
tish_native_times=()
tish_cranelift_times=()
tish_llvm_times=()
tish_wasi_times=()
node_times=()

want_runtime vm && run_with_timeout $tish_bin run "$entry_tish" --backend vm >/dev/null 2>&1 || true
want_runtime interp && run_with_timeout $tish_bin run "$entry_tish" --backend interp >/dev/null 2>&1 || true
want_runtime rust && [[ -x "$native_bin" ]] && run_with_timeout "$native_bin" >/dev/null 2>&1 || true
want_runtime cranelift && [[ -x "$cranelift_bin" ]] && run_with_timeout "$cranelift_bin" >/dev/null 2>&1 || true
want_runtime llvm && [[ -x "$llvm_bin" ]] && run_with_timeout "$llvm_bin" >/dev/null 2>&1 || true
want_runtime wasi && $has_wasmtime && [[ -f "$wasi_bin" ]] && run_with_timeout wasmtime --dir /tmp "$wasi_bin" >/dev/null 2>&1 || true
want_runtime node && "$node_cmd" "$js_file" >/dev/null 2>&1 || true

if want_runtime vm; then
  for _ in $(seq 1 "$n"); do
    t0=$(ms)
    run_with_timeout $tish_bin run "$entry_tish" --backend vm >/dev/null 2>&1 || true
    t1=$(ms)
    tish_vm_times+=($((t1 - t0)))
  done
fi
if want_runtime interp; then
  for _ in $(seq 1 "$n"); do
    t0=$(ms)
    run_with_timeout $tish_bin run "$entry_tish" --backend interp >/dev/null 2>&1 || true
    t1=$(ms)
    tish_interp_times+=($((t1 - t0)))
  done
fi
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
wasi_ok=false
if want_runtime wasi && $has_wasmtime && [[ -f "$wasi_bin" ]]; then
  wasi_ok=true
  for _ in $(seq 1 "$n"); do
    t0=$(ms)
    run_with_timeout wasmtime --dir /tmp "$wasi_bin" >/dev/null 2>&1 || true
    t1=$(ms)
    tish_wasi_times+=($((t1 - t0)))
  done
fi
if want_runtime node; then
  for _ in $(seq 1 "$n"); do
    t0=$(ms)
    "$node_cmd" "$js_file" >/dev/null 2>&1 || true
    t1=$(ms)
    node_times+=($((t1 - t0)))
  done
fi

tish_vm_sum=0
for t in "${tish_vm_times[@]+"${tish_vm_times[@]}"}"; do tish_vm_sum=$((tish_vm_sum + t)); done
tish_interp_sum=0
for t in "${tish_interp_times[@]+"${tish_interp_times[@]}"}"; do tish_interp_sum=$((tish_interp_sum + t)); done
tish_native_sum=0
for t in "${tish_native_times[@]+"${tish_native_times[@]}"}"; do tish_native_sum=$((tish_native_sum + t)); done
tish_cranelift_sum=0
for t in "${tish_cranelift_times[@]+"${tish_cranelift_times[@]}"}"; do tish_cranelift_sum=$((tish_cranelift_sum + t)); done
tish_llvm_sum=0
for t in "${tish_llvm_times[@]+"${tish_llvm_times[@]}"}"; do tish_llvm_sum=$((tish_llvm_sum + t)); done
tish_wasi_sum=0
for t in "${tish_wasi_times[@]+"${tish_wasi_times[@]}"}"; do tish_wasi_sum=$((tish_wasi_sum + t)); done
node_sum=0
for t in "${node_times[@]+"${node_times[@]}"}"; do node_sum=$((node_sum + t)); done

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
ratio=0
[[ $node_avg -gt 0 ]] && ratio=$((tish_vm_avg * 100 / node_avg))

echo ""
echo "══════════════════════════════════════════════════════════════"
echo "  BUNDLED PERF SUITE (${n}-run average, ms)"
echo "══════════════════════════════════════════════════════════════"
want_runtime vm && echo "  Tish (vm):        ${tish_vm_avg}ms"
want_runtime interp && echo "  Tish (interp):    ${tish_interp_avg}ms"
want_runtime rust && $compile_ok && echo "  Tish (rust):      ${tish_native_avg}ms"
want_runtime cranelift && $cranelift_ok && echo "  Tish (cranelift): ${tish_cranelift_avg}ms"
want_runtime llvm && $llvm_ok && echo "  Tish (llvm):      ${tish_llvm_avg}ms"
want_runtime wasi && $wasi_ok && echo "  Tish (wasi):      ${tish_wasi_avg}ms"
want_runtime node && echo "  Node.js:          ${node_avg}ms"
want_runtime vm && want_runtime node && echo "  Tish(vm)/Node:    ${ratio}%"
echo ""

if $github_step_summary && [[ -n "${GITHUB_STEP_SUMMARY:-}" ]]; then
  {
    echo "## Bundled perf suite (\`tests/main.tish\`)"
    echo ""
    echo "| Runtime | Avg (ms) |"
    echo "|---------|----------|"
    want_runtime vm && echo "| vm | ${tish_vm_avg} |"
    want_runtime interp && echo "| interp | ${tish_interp_avg} |"
    want_runtime rust && $compile_ok && echo "| rust native | ${tish_native_avg} |"
    want_runtime cranelift && $cranelift_ok && echo "| cranelift | ${tish_cranelift_avg} |"
    want_runtime llvm && $llvm_ok && echo "| llvm | ${tish_llvm_avg} |"
    want_runtime wasi && $wasi_ok && echo "| wasi (wasmtime) | ${tish_wasi_avg} |"
    want_runtime node && echo "| Node | ${node_avg} |"
    want_runtime vm && want_runtime node && echo "| **Tish(vm)/Node %** | **${ratio}** |"
    echo ""
    echo "Profile: \`${profile}\`. ${n} timed runs after warmup; timeout ${run_timeout}s per invocation."
  } >> "$GITHUB_STEP_SUMMARY"
fi

if [[ "${PERF_SUITE_STRICT:-}" == "1" ]]; then
  if want_runtime rust && ! $compile_ok; then
    echo "ERROR: PERF_SUITE_STRICT=1 but rust native binary missing or failed to build"
    exit 1
  fi
  if want_runtime cranelift && ! $cranelift_ok; then
    echo "ERROR: PERF_SUITE_STRICT=1 but cranelift binary missing or failed to build"
    exit 1
  fi
  if want_runtime llvm && ! $llvm_ok; then
    echo "ERROR: PERF_SUITE_STRICT=1 but llvm binary missing or failed to build"
    exit 1
  fi
  if want_runtime wasi && $has_wasmtime && ! $wasi_ok; then
    echo "ERROR: PERF_SUITE_STRICT=1 but wasi build failed or wasm missing"
    exit 1
  fi
fi

echo "Done."
