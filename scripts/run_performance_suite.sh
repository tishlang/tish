#!/bin/bash
# Bundled perf suite: per-test timings (each paired .tish) + whole-program tests/main.tish (+ main.js for JS runtimes); one native binary per backend for the bundle.
# Regenerate sources: ./scripts/generate_perf_ci_main.sh
# Usage: ./scripts/run_performance_suite.sh [--release] [--summary-only] [--no-compile] [--timeout SEC] [--filter NAME] [--runtimes R,...] [--verbose]
#   --filter NAME   run only per-file micro-tests whose tid contains NAME (e.g. array_stress, modules, modules/promise); does not skip bundled tests/main.tish
#   --runtimes defaults to all: vm,interp,rust,cranelift,llvm,wasi,node,bun,deno,qjs (bun/deno/qjs if installed)
#   Default: same as run_performance_manual.sh — for each micro-test and for the full bundle, print program
#   stdout/stderr (console.log, etc.), then "${n}-run avg" timing lines, then the sorted table + bundle summary.
#   Failures: non-zero exit, empty/whitespace-only captured output, or error-like text → red >>> FAILED;
#   end summary counts micro-tests (one per paired tid) + one bundle row (display + timed invocations).
#   --summary-only          skips per-file program output only; bundled rust/cflt/llvm/wasi still run here.
#   --verbose               do not suppress stderr during timed runs (crash logs from timeout wrapper).
#   --github-step-summary   append markdown to GITHUB_STEP_SUMMARY (CI)
#
# For per-file-only runs under tests/core and tests/modules without the bundled main, use ./scripts/run_performance_manual.sh

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
filter_name=""
github_step_summary=false

while [[ $# -gt 0 ]]; do
  case "$1" in
    --release) profile="release"; shift ;;
    --summary-only) summary_only=true; shift ;;
    --no-compile) no_compile=true; shift ;;
    --timeout) run_timeout="$2"; shift 2 ;;
    --filter) filter_name="$2"; shift 2 ;;
    --runtimes) runtimes_filter="$2"; shift 2 ;;
    --verbose|-v) verbose=true; shift ;;
    --github-step-summary) github_step_summary=true; shift ;;
    *) shift ;;
  esac
done

n=5

# ── exit / stderr / error-text detection (for red FAIL + failure %) ─────────
PERF_RED=$'\033[1;31m'
PERF_GRN=$'\033[0;32m'
PERF_DIM=$'\033[0;90m'
PERF_RST=$'\033[0m'
perf_use_color=false
[[ -t 1 ]] && perf_use_color=true
_perf_r() { [ "$perf_use_color" = true ] && printf %s "$PERF_RED" || true; }
_perf_g() { [ "$perf_use_color" = true ] && printf %s "$PERF_GRN" || true; }
_perf_d() { [ "$perf_use_color" = true ] && printf %s "$PERF_DIM" || true; }
_perf_x() { [ "$perf_use_color" = true ] && printf %s "$PERF_RST" || true; }

# True if log file + exit code indicate failure (stderr is merged into log when we capture with 2>&1).
# Exit 0 with empty or whitespace-only captured output counts as failure (silent success is suspicious for perf fixtures).
_perf_has_error_signal() {
  local log="$1" ec="$2"
  [[ -z "$log" || ! -f "$log" ]] && return 0
  [[ "$ec" -ne 0 ]] && return 0
  if ! [[ -s "$log" ]] || ! grep -qE '[^[:space:]]' "$log" 2>/dev/null; then
    return 0
  fi
  if grep -qiE \
    '(error|Error:|ERROR:|panic|Panic|PANIC|Uncaught|Unhandled|Exception:|fatal:|FATAL|stack[[:space:]]+backtrace|CompileError|ReferenceError|SyntaxError|TypeError|TishError|Throw:|failed with|exited with code [1-9])' \
    "$log" 2>/dev/null; then
    return 0
  fi
  return 1
}

# Run with wall-clock timeout; preserve exit code (no trailing "|| true"). Stderr: shown only if [ "$verbose" = true ].
_rtc() {
  local ec=0
  if [[ $run_timeout -le 0 ]]; then
    if [ "$verbose" = true ]; then "$@"; else "$@" 2>&1; fi
    ec=$?
    return "$ec"
  fi
  if command -v timeout &>/dev/null; then
    if [ "$verbose" = true ]; then timeout "$run_timeout" "$@"; else timeout "$run_timeout" "$@" 2>&1; fi
    ec=$?
    return "$ec"
  fi
  if command -v gtimeout &>/dev/null; then
    if [ "$verbose" = true ]; then gtimeout "$run_timeout" "$@"; else gtimeout "$run_timeout" "$@" 2>&1; fi
    ec=$?
    return "$ec"
  fi
  if command -v perl &>/dev/null; then
    if [ "$verbose" = true ]; then
      perl -e '
        my $t=shift;
        my $pid=fork;
        die "fork: $!" if !defined $pid;
        if ($pid==0) { setpgrp(0,0) if defined &setpgrp; exec(@ARGV) or exit 127; }
        $SIG{ALRM}=sub{ kill 9,-$pid; kill 9,$pid; waitpid $pid,0; exit 124 };
        alarm $t; waitpid $pid,0; alarm 0; exit($?>>8)
      ' "$run_timeout" "$@"
      ec=$?
    else
      perl -e '
        my $t=shift;
        my $pid=fork;
        die "fork: $!" if !defined $pid;
        if ($pid==0) { setpgrp(0,0) if defined &setpgrp; exec(@ARGV) or exit 127; }
        $SIG{ALRM}=sub{ kill 9,-$pid; kill 9,$pid; waitpid $pid,0; exit 124 };
        alarm $t; waitpid $pid,0; alarm 0; exit($?>>8)
      ' "$run_timeout" "$@" 2>&1
      ec=$?
    fi
    return "$ec"
  fi
  set +e
  set -m
  "$@" 2>&1 & pid=$!
  ( sleep "$run_timeout"; kill -TERM -"$pid" 2>/dev/null; sleep 2; kill -KILL -"$pid" 2>/dev/null ) & k=$!
  wait "$pid" 2>/dev/null
  ec=$?
  kill "$k" 2>/dev/null
  wait "$k" 2>/dev/null
  set -e
  return "$ec"
}

# Print merged stdout+stderr, then OK/FAIL line. Sets perf_last_subfail=0|1.
_perf_run_show() {
  local label="$1" ec=0
  shift
  local log
  log=$(mktemp)
  set +e
  if [[ $run_timeout -le 0 ]]; then
    "$@" >"$log" 2>&1
    ec=$?
  elif command -v timeout &>/dev/null; then
    timeout "$run_timeout" "$@" >"$log" 2>&1
    ec=$?
  elif command -v gtimeout &>/dev/null; then
    gtimeout "$run_timeout" "$@" >"$log" 2>&1
    ec=$?
  elif command -v perl &>/dev/null; then
    perl -e '
      my $t = shift;
      my $log = shift;
      my @cmd = @ARGV;
      my $pid = fork;
      die "fork: $!" if !defined $pid;
      if ($pid == 0) {
        open STDOUT, ">", $log or exit 126;
        open STDERR, ">&", \*STDOUT or exit 126;
        setpgrp(0, 0) if defined &setpgrp;
        exec(@cmd) or exit 127;
      }
      $SIG{ALRM} = sub { kill 9, -$pid; kill 9, $pid; waitpid $pid, 0; exit 124 };
      alarm $t;
      waitpid $pid, 0;
      alarm 0;
      exit($? >> 8)
    ' "$run_timeout" "$log" "$@"
    ec=$?
  else
    "$@" >"$log" 2>&1 &
    local pid=$!
    ( sleep "$run_timeout"; kill -TERM "$pid" 2>/dev/null; sleep 2; kill -KILL "$pid" 2>/dev/null ) &
    local k=$!
    wait "$pid" 2>/dev/null
    ec=$?
    kill "$k" 2>/dev/null
    wait "$k" 2>/dev/null
  fi
  set -e
  if [[ -s "$log" ]]; then
    cat "$log"
  else
    _perf_d
    echo "(no bytes on stdout/stderr)"
    _perf_x
  fi
  perf_last_subfail=0
  if _perf_has_error_signal "$log" "$ec"; then
    perf_last_subfail=1
    _perf_r
    if [[ "$ec" -eq 0 ]] && { [[ ! -s "$log" ]] || ! grep -qE '[^[:space:]]' "$log" 2>/dev/null; }; then
      echo ">>> FAILED: $label (exit=$ec, empty or whitespace-only output)"
    else
      echo ">>> FAILED: $label (exit=$ec)"
    fi
    _perf_x
  else
    _perf_g
    echo ">>> OK: $label (exit=$ec)"
    _perf_x
  fi
  rm -f "$log"
}

# Discard child output; return non-zero if child failed or timed out (used for warmups + avg loops).
_perf_rtc_discard() {
  set +e
  if [ "$verbose" = true ]; then
    _rtc "$@" >/dev/null
  else
    _rtc "$@" >/dev/null 2>&1
  fi
  local ec=$?
  set -e
  return "$ec"
}

# Micro timing table: which tid failed which runtime (one tid per line, deduped).
_micro_mark_col() {
  local f="$1" t="$2"
  [[ -n "$f" && -n "$t" && -f "$f" ]] || return
  grep -Fxq "$t" "$f" 2>/dev/null || echo "$t" >>"$f"
}
_micro_col_has() {
  [[ -n "$1" && -n "$2" && -f "$2" ]] && grep -Fxq "$1" "$2" 2>/dev/null
}
# Right-aligned 7-char numeric cell; red when fail=1 (TTY colors only affect display).
_perf_tbl_int7() {
  if [[ "$1" -eq 1 ]]; then
    _perf_r
    printf '%7d' "$2"
    _perf_x
  else
    printf '%7d' "$2"
  fi
}

# Counters: micro-test rows (each tid) and bundle display checks
perf_micro_total=0
perf_micro_failed=0
perf_bundle_checks=0
perf_bundle_failed=0

run_with_timeout() {
  if [[ $run_timeout -le 0 ]]; then
    if [ "$verbose" = true ]; then "$@" || true; else "$@" 2>/dev/null || true; fi
    return
  fi
  if command -v timeout &>/dev/null; then
    if [ "$verbose" = true ]; then timeout "$run_timeout" "$@" || true; else timeout "$run_timeout" "$@" 2>/dev/null || true; fi
    return
  fi
  if command -v gtimeout &>/dev/null; then
    if [ "$verbose" = true ]; then gtimeout "$run_timeout" "$@" || true; else gtimeout "$run_timeout" "$@" 2>/dev/null || true; fi
    return
  fi
  if command -v perl &>/dev/null; then
    if [ "$verbose" = true ]; then
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
  ( set +e; set -m; "$@" 2>/dev/null & pid=$!; ( sleep "$run_timeout"; kill -TERM "-$pid" 2>/dev/null; sleep 2; kill -KILL "-$pid" 2>/dev/null ) & k=$!; wait "$pid" 2>/dev/null; kill "$k" 2>/dev/null; wait "$k" 2>/dev/null ) || true
}

tish_bin="$target_dir/$profile/tish"
rel_flag=""
[[ "$profile" == "release" ]] && rel_flag="--release"

if [[ ! -f "$entry_tish" ]] || [[ ! -f "$entry_js" ]]; then
  echo "Missing $entry_tish or $entry_js — run: ./scripts/generate_perf_ci_main.sh"
  exit 1
fi

echo "Building tish ($profile)..."
# Avoid split target dirs (e.g. CARGO_TARGET_DIR in some IDEs): workspace tish must match freshly built tishlang_compile.
( unset CARGO_TARGET_DIR; cargo build -p tishlang $rel_flag --features full --target-dir "$target_dir" -q 2>/dev/null ) || true
if [[ ! -x "$tish_bin" ]]; then
  tish_bin="cargo run -p tishlang $rel_flag --features full --target-dir $target_dir -q --"
fi

cache_dir="$target_dir/perf-suite-cache-$profile"
if [ "$no_compile" = true ]; then
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

# n-run average wall ms; sets _avg_run_last_ms; ORs perf_last_avg_fail=1 if any timed invocation exits non-zero.
# Must be called in the current shell (not $(...)) so perf_last_avg_fail updates are visible.
_avg_run_ms() {
  local sum=0 _ t0 t1 ec
  for _ in $(seq 1 "$n"); do
    t0=$(ms)
    set +e
    if [ "$verbose" = true ]; then
      _rtc "$@" >/dev/null
    else
      _rtc "$@" >/dev/null 2>&1
    fi
    ec=$?
    set -e
    [[ $ec -ne 0 ]] && perf_last_avg_fail=1
    t1=$(ms)
    sum=$((sum + t1 - t0))
  done
  _avg_run_last_ms=$((sum / n))
}

# Last per-test average (ms) and whether warmup or any timed run failed (for set -u callers; never use $(...)).
_pt_avg_last_ms=0
_pt_avg_last_fail=0

_pt_avg_vm() {
  local f="$1"
  if ! want_runtime vm; then
    _pt_avg_last_ms=0
    _pt_avg_last_fail=0
    return
  fi
  perf_last_avg_fail=0
  _perf_rtc_discard "$tish_bin" run "$f" --backend vm || perf_last_avg_fail=1
  _avg_run_ms "$tish_bin" run "$f" --backend vm
  _pt_avg_last_ms=$_avg_run_last_ms
  _pt_avg_last_fail=$perf_last_avg_fail
}
_pt_avg_interp() {
  local f="$1"
  if ! want_runtime interp; then
    _pt_avg_last_ms=0
    _pt_avg_last_fail=0
    return
  fi
  perf_last_avg_fail=0
  _perf_rtc_discard "$tish_bin" run "$f" --backend interp || perf_last_avg_fail=1
  _avg_run_ms "$tish_bin" run "$f" --backend interp
  _pt_avg_last_ms=$_avg_run_last_ms
  _pt_avg_last_fail=$perf_last_avg_fail
}
_pt_avg_node() {
  local j="$1" sum=0 _ t0 t1 ec
  if ! want_runtime node; then
    _pt_avg_last_ms=0
    _pt_avg_last_fail=0
    return
  fi
  perf_last_avg_fail=0
  set +e
  if ! "$node_cmd" "$j" >/dev/null 2>&1; then perf_last_avg_fail=1; fi
  set -e
  for _ in $(seq 1 "$n"); do
    t0=$(ms)
    set +e
    "$node_cmd" "$j" >/dev/null 2>&1
    ec=$?
    set -e
    [[ $ec -ne 0 ]] && perf_last_avg_fail=1
    t1=$(ms)
    sum=$((sum + t1 - t0))
  done
  _pt_avg_last_ms=$((sum / n))
  _pt_avg_last_fail=$perf_last_avg_fail
}
_pt_avg_bun() {
  local j="$1" sum=0 _ t0 t1 ec
  if ! want_runtime bun || [ "$has_bun" != true ]; then
    _pt_avg_last_ms=0
    _pt_avg_last_fail=0
    return
  fi
  perf_last_avg_fail=0
  set +e
  if ! "$bun_cmd" "$j" >/dev/null 2>&1; then perf_last_avg_fail=1; fi
  set -e
  for _ in $(seq 1 "$n"); do
    t0=$(ms)
    set +e
    "$bun_cmd" "$j" >/dev/null 2>&1
    ec=$?
    set -e
    [[ $ec -ne 0 ]] && perf_last_avg_fail=1
    t1=$(ms)
    sum=$((sum + t1 - t0))
  done
  _pt_avg_last_ms=$((sum / n))
  _pt_avg_last_fail=$perf_last_avg_fail
}
_pt_avg_deno() {
  local j="$1" sum=0 _ t0 t1 ec
  if ! want_runtime deno || [ "$has_deno" != true ]; then
    _pt_avg_last_ms=0
    _pt_avg_last_fail=0
    return
  fi
  perf_last_avg_fail=0
  set +e
  if ! "$deno_cmd" run --allow-all "$j" >/dev/null 2>&1; then perf_last_avg_fail=1; fi
  set -e
  for _ in $(seq 1 "$n"); do
    t0=$(ms)
    set +e
    "$deno_cmd" run --allow-all "$j" >/dev/null 2>&1
    ec=$?
    set -e
    [[ $ec -ne 0 ]] && perf_last_avg_fail=1
    t1=$(ms)
    sum=$((sum + t1 - t0))
  done
  _pt_avg_last_ms=$((sum / n))
  _pt_avg_last_fail=$perf_last_avg_fail
}
_pt_avg_qjs() {
  local j="$1" sum=0 _ t0 t1 ec
  if ! want_runtime qjs || [ "$has_qjs" != true ]; then
    _pt_avg_last_ms=0
    _pt_avg_last_fail=0
    return
  fi
  perf_last_avg_fail=0
  set +e
  if ! "$qjs_cmd" "$j" >/dev/null 2>&1; then perf_last_avg_fail=1; fi
  set -e
  for _ in $(seq 1 "$n"); do
    t0=$(ms)
    set +e
    "$qjs_cmd" "$j" >/dev/null 2>&1
    ec=$?
    set -e
    [[ $ec -ne 0 ]] && perf_last_avg_fail=1
    t1=$(ms)
    sum=$((sum + t1 - t0))
  done
  _pt_avg_last_ms=$((sum / n))
  _pt_avg_last_fail=$perf_last_avg_fail
}

# One visible run per runtime for the bundled entry (matches run_performance_manual.sh sections).
_show_suite_bundle_output() {
  if want_runtime vm; then
    echo "Tish (vm):"
    perf_bundle_checks=$((perf_bundle_checks + 1))
    _perf_run_show "bundle vm ($entry_tish)" "$tish_bin" run "$entry_tish" --backend vm
    [[ $perf_last_subfail -eq 1 ]] && perf_bundle_failed=$((perf_bundle_failed + 1))
    echo ""
  fi
  if want_runtime interp; then
    echo "Tish (interp):"
    perf_bundle_checks=$((perf_bundle_checks + 1))
    _perf_run_show "bundle interp ($entry_tish)" "$tish_bin" run "$entry_tish" --backend interp
    [[ $perf_last_subfail -eq 1 ]] && perf_bundle_failed=$((perf_bundle_failed + 1))
    echo ""
  fi
  if want_runtime rust && [[ -x "$native_bin" ]]; then
    echo "Tish (rust):"
    perf_bundle_checks=$((perf_bundle_checks + 1))
    _perf_run_show "bundle rust native" "$native_bin"
    [[ $perf_last_subfail -eq 1 ]] && perf_bundle_failed=$((perf_bundle_failed + 1))
    echo ""
  elif want_runtime rust; then
    perf_bundle_checks=$((perf_bundle_checks + 1))
    perf_bundle_failed=$((perf_bundle_failed + 1))
    _perf_r
    echo "Tish (rust): (not built — requested but binary missing; counts as failure)"
    _perf_x
    echo ""
  fi
  if want_runtime cranelift && [[ -x "$cranelift_bin" ]]; then
    echo "Tish (cranelift):"
    perf_bundle_checks=$((perf_bundle_checks + 1))
    _perf_run_show "bundle cranelift" "$cranelift_bin"
    [[ $perf_last_subfail -eq 1 ]] && perf_bundle_failed=$((perf_bundle_failed + 1))
    echo ""
  elif want_runtime cranelift; then
    perf_bundle_checks=$((perf_bundle_checks + 1))
    perf_bundle_failed=$((perf_bundle_failed + 1))
    _perf_r
    echo "Tish (cranelift): (not built — requested but binary missing; counts as failure)"
    _perf_x
    echo ""
  fi
  if want_runtime llvm && [[ -x "$llvm_bin" ]]; then
    echo "Tish (llvm):"
    perf_bundle_checks=$((perf_bundle_checks + 1))
    _perf_run_show "bundle llvm" "$llvm_bin"
    [[ $perf_last_subfail -eq 1 ]] && perf_bundle_failed=$((perf_bundle_failed + 1))
    echo ""
  elif want_runtime llvm; then
    perf_bundle_checks=$((perf_bundle_checks + 1))
    perf_bundle_failed=$((perf_bundle_failed + 1))
    _perf_r
    echo "Tish (llvm): (not built — requested but binary missing; counts as failure)"
    _perf_x
    echo ""
  fi
  if want_runtime wasi; then
    echo "Tish (wasi):"
    if [ "$has_wasmtime" = true ] && [[ -f "$wasi_bin" ]]; then
      perf_bundle_checks=$((perf_bundle_checks + 1))
      _perf_run_show "bundle wasi (wasmtime)" wasmtime --dir /tmp "$wasi_bin"
      [[ $perf_last_subfail -eq 1 ]] && perf_bundle_failed=$((perf_bundle_failed + 1))
    else
      perf_bundle_checks=$((perf_bundle_checks + 1))
      perf_bundle_failed=$((perf_bundle_failed + 1))
      _perf_r
      echo "(not built or wasmtime not found — counts as failure)"
      _perf_x
    fi
    echo ""
  fi
  if want_runtime node; then
    echo "Node.js:"
    perf_bundle_checks=$((perf_bundle_checks + 1))
    _perf_run_show "bundle Node ($js_file)" "$node_cmd" "$js_file"
    [[ $perf_last_subfail -eq 1 ]] && perf_bundle_failed=$((perf_bundle_failed + 1))
    echo ""
  fi
  if want_runtime bun && "$has_bun"&& "$has_bun" [ "$has_bun" = true ]; then
    echo "Bun:"
    perf_bundle_checks=$((perf_bundle_checks + 1))
    _perf_run_show "bundle Bun ($js_file)" "$bun_cmd" "$js_file"
    [[ $perf_last_subfail -eq 1 ]] && perf_bundle_failed=$((perf_bundle_failed + 1))
    echo ""
  fi
  if want_runtime deno && "$has_deno"&& "$has_deno" [ "$has_deno" = true ]; then
    echo "Deno:"
    perf_bundle_checks=$((perf_bundle_checks + 1))
    _perf_run_show "bundle Deno ($js_file)" "$deno_cmd" run --allow-all "$js_file"
    [[ $perf_last_subfail -eq 1 ]] && perf_bundle_failed=$((perf_bundle_failed + 1))
    echo ""
  fi
  if want_runtime qjs && "$has_qjs"&& "$has_qjs" [ "$has_qjs" = true ]; then
    echo "QuickJS:"
    perf_bundle_checks=$((perf_bundle_checks + 1))
    _perf_run_show "bundle QuickJS ($js_file)" "$qjs_cmd" "$js_file"
    [[ $perf_last_subfail -eq 1 ]] && perf_bundle_failed=$((perf_bundle_failed + 1))
    echo ""
  fi
}

cache_key="ci_main_suite"
native_bin="$compile_dir/${cache_key}_native"
cranelift_bin="$compile_dir/${cache_key}_cranelift"
llvm_bin="$compile_dir/${cache_key}_llvm"
wasi_bin="$compile_dir/${cache_key}_wasi.wasm"
js_file="$entry_js"

echo "=== Tish bundled perf suite ($test_id) ==="
echo "Profile: $profile"
echo "Entry: $entry_tish"
[[ -n "${runtimes_filter:-}" ]] && echo "Runtimes: $runtimes_filter (use --runtimes vm,interp,rust,cranelift,llvm,wasi,node,bun,deno,qjs)"
[[ $run_timeout -gt 0 ]] && echo "Timeout per run: ${run_timeout}s"
echo ""
echo "Runtimes to test:"
want_runtime vm && echo "  vm (tish run --backend vm)"
want_runtime interp && echo "  interp (tish run --backend interp)"
want_runtime rust && echo "  rust (tish native)"
want_runtime cranelift && echo "  cranelift (tish JIT)"
want_runtime llvm && echo "  llvm (tish native via clang)"
want_runtime wasi && echo "  wasi (wasmtime)"
want_runtime node && echo "  node"
want_runtime bun && "$has_bun"&& "$has_bun" [ "$has_bun" = true ] && echo "  bun"
want_runtime deno && "$has_deno"&& "$has_deno" [ "$has_deno" = true ] && echo "  deno"
want_runtime qjs && "$has_qjs"&& "$has_qjs" [ "$has_qjs" = true ] && echo "  qjs"
[[ -n "${filter_name:-}" ]] && echo "Filter (per-file micro-tests only): $filter_name"
echo ""

if [ "$no_compile" != true ]; then
  echo "Compiling suite (rust / cranelift / llvm / wasi)..."
  echo -n "  $test_id: "
  _suite_build_log=$(mktemp)
  if want_runtime rust; then
    if "$tish_bin" build "$entry_tish" -o "$native_bin" --native-backend rust >"$_suite_build_log" 2>&1; then
      echo -n "rust "
    else
      echo -n "rust-fail "
      if [[ -s "$_suite_build_log" ]]; then
        cp "$_suite_build_log" "$compile_dir/last_rust_native_build.log" 2>/dev/null || true
      fi
      if [ "$summary_only" != true ] && [[ -s "$_suite_build_log" ]]; then
        echo ""
        echo "  (rust native build — last 50 lines:)"
        tail -50 "$_suite_build_log" | sed 's/^/  | /'
        echo ""
      fi
    fi
  fi
  if want_runtime cranelift; then
    if "$tish_bin" build "$entry_tish" -o "$cranelift_bin" --native-backend cranelift >"$_suite_build_log" 2>&1; then
      echo -n "cranelift "
    else
      echo -n "cranelift-fail "
      if [ "$summary_only" != true ] && [[ -s "$_suite_build_log" ]]; then
        echo ""
        echo "  (cranelift build — last 50 lines:)"
        tail -50 "$_suite_build_log" | sed 's/^/  | /'
        echo ""
      fi
    fi
  fi
  if want_runtime llvm; then
    if "$tish_bin" build "$entry_tish" -o "$llvm_bin" --native-backend llvm >"$_suite_build_log" 2>&1; then
      echo -n "llvm "
    else
      echo -n "llvm-fail "
      if [ "$summary_only" != true ] && [[ -s "$_suite_build_log" ]]; then
        echo ""
        echo "  (llvm build — last 50 lines:)"
        tail -50 "$_suite_build_log" | sed 's/^/  | /'
        echo ""
      fi
    fi
  fi
  if want_runtime wasi; then
    if [ "$has_wasmtime" = true ]; then
      if "$tish_bin" build "$entry_tish" -o "$compile_dir/${cache_key}_wasi" --target wasi >"$_suite_build_log" 2>&1; then
        echo -n "wasi"
      else
        echo -n "wasi-fail"
        if [ "$summary_only" != true ] && [[ -s "$_suite_build_log" ]]; then
          echo ""
          echo "  (wasi build — last 50 lines:)"
          tail -50 "$_suite_build_log" | sed 's/^/  | /'
          echo ""
        fi
      fi
    else
      echo -n "wasi-skip"
    fi
  fi
  rm -f "$_suite_build_log"
  echo ""
  echo ""
fi

# Full-bundle timings (tests/main.*): used for summary, TOTAL-row native columns, and vm/Node%.
# Runs before per-test micro-runs so the table can show bundle averages on the TOTAL line.
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

perf_bundle_timing_fail=0
want_runtime vm && { _perf_rtc_discard "$tish_bin" run "$entry_tish" --backend vm || perf_bundle_timing_fail=1; }
want_runtime interp && { _perf_rtc_discard "$tish_bin" run "$entry_tish" --backend interp || perf_bundle_timing_fail=1; }
want_runtime rust && [[ -x "$native_bin" ]] && { _perf_rtc_discard "$native_bin" || perf_bundle_timing_fail=1; }
want_runtime cranelift && [[ -x "$cranelift_bin" ]] && { _perf_rtc_discard "$cranelift_bin" || perf_bundle_timing_fail=1; }
want_runtime llvm && [[ -x "$llvm_bin" ]] && { _perf_rtc_discard "$llvm_bin" || perf_bundle_timing_fail=1; }
want_runtime wasi && "$has_wasmtime"&& "$has_wasmtime" [ "$has_wasmtime" = true ] && [[ -f "$wasi_bin" ]] && { _perf_rtc_discard wasmtime --dir /tmp "$wasi_bin" || perf_bundle_timing_fail=1; }
want_runtime node && { _perf_rtc_discard "$node_cmd" "$js_file" || perf_bundle_timing_fail=1; }
want_runtime bun && "$has_bun"&& "$has_bun" [ "$has_bun" = true ] && { _perf_rtc_discard "$bun_cmd" "$js_file" || perf_bundle_timing_fail=1; }
want_runtime deno && "$has_deno"&& "$has_deno" [ "$has_deno" = true ] && { _perf_rtc_discard "$deno_cmd" run --allow-all "$js_file" || perf_bundle_timing_fail=1; }
want_runtime qjs && "$has_qjs"&& "$has_qjs" [ "$has_qjs" = true ] && { _perf_rtc_discard "$qjs_cmd" "$js_file" || perf_bundle_timing_fail=1; }

if want_runtime vm; then
  for _ in $(seq 1 "$n"); do
    t0=$(ms)
    _perf_rtc_discard "$tish_bin" run "$entry_tish" --backend vm || perf_bundle_timing_fail=1
    t1=$(ms)
    tish_vm_times+=($((t1 - t0)))
  done
fi
if want_runtime interp; then
  for _ in $(seq 1 "$n"); do
    t0=$(ms)
    _perf_rtc_discard "$tish_bin" run "$entry_tish" --backend interp || perf_bundle_timing_fail=1
    t1=$(ms)
    tish_interp_times+=($((t1 - t0)))
  done
fi
compile_ok=false
if want_runtime rust && [[ -x "$native_bin" ]]; then
  compile_ok=true
  for _ in $(seq 1 "$n"); do
    t0=$(ms)
    _perf_rtc_discard "$native_bin" || perf_bundle_timing_fail=1
    t1=$(ms)
    tish_native_times+=($((t1 - t0)))
  done
fi
cranelift_ok=false
if want_runtime cranelift && [[ -x "$cranelift_bin" ]]; then
  cranelift_ok=true
  for _ in $(seq 1 "$n"); do
    t0=$(ms)
    _perf_rtc_discard "$cranelift_bin" || perf_bundle_timing_fail=1
    t1=$(ms)
    tish_cranelift_times+=($((t1 - t0)))
  done
fi
llvm_ok=false
if want_runtime llvm && [[ -x "$llvm_bin" ]]; then
  llvm_ok=true
  for _ in $(seq 1 "$n"); do
    t0=$(ms)
    _perf_rtc_discard "$llvm_bin" || perf_bundle_timing_fail=1
    t1=$(ms)
    tish_llvm_times+=($((t1 - t0)))
  done
fi
wasi_ok=false
if want_runtime wasi && "$has_wasmtime"&& "$has_wasmtime" [ "$has_wasmtime" = true ] && [[ -f "$wasi_bin" ]]; then
  wasi_ok=true
  for _ in $(seq 1 "$n"); do
    t0=$(ms)
    _perf_rtc_discard wasmtime --dir /tmp "$wasi_bin" || perf_bundle_timing_fail=1
    t1=$(ms)
    tish_wasi_times+=($((t1 - t0)))
  done
fi
if want_runtime node; then
  for _ in $(seq 1 "$n"); do
    t0=$(ms)
    _perf_rtc_discard "$node_cmd" "$js_file" || perf_bundle_timing_fail=1
    t1=$(ms)
    node_times+=($((t1 - t0)))
  done
fi
if want_runtime bun && "$has_bun"&& "$has_bun" [ "$has_bun" = true ]; then
  for _ in $(seq 1 "$n"); do
    t0=$(ms)
    _perf_rtc_discard "$bun_cmd" "$js_file" || perf_bundle_timing_fail=1
    t1=$(ms)
    bun_times+=($((t1 - t0)))
  done
fi
if want_runtime deno && "$has_deno"&& "$has_deno" [ "$has_deno" = true ]; then
  for _ in $(seq 1 "$n"); do
    t0=$(ms)
    _perf_rtc_discard "$deno_cmd" run --allow-all "$js_file" || perf_bundle_timing_fail=1
    t1=$(ms)
    deno_times+=($((t1 - t0)))
  done
fi
if want_runtime qjs && "$has_qjs"&& "$has_qjs" [ "$has_qjs" = true ]; then
  for _ in $(seq 1 "$n"); do
    t0=$(ms)
    _perf_rtc_discard "$qjs_cmd" "$js_file" || perf_bundle_timing_fail=1
    t1=$(ms)
    qjs_times+=($((t1 - t0)))
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
bun_sum=0
for t in "${bun_times[@]+"${bun_times[@]}"}"; do bun_sum=$((bun_sum + t)); done
deno_sum=0
for t in "${deno_times[@]+"${deno_times[@]}"}"; do deno_sum=$((deno_sum + t)); done
qjs_sum=0
for t in "${qjs_times[@]+"${qjs_times[@]}"}"; do qjs_sum=$((qjs_sum + t)); done

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
want_runtime bun && "$has_bun"&& "$has_bun" [ "$has_bun" = true ] && [[ ${#bun_times[@]} -eq $n ]] && bun_avg=$((bun_sum / n))
want_runtime deno && "$has_deno"&& "$has_deno" [ "$has_deno" = true ] && [[ ${#deno_times[@]} -eq $n ]] && deno_avg=$((deno_sum / n))
want_runtime qjs && "$has_qjs"&& "$has_qjs" [ "$has_qjs" = true ] && [[ ${#qjs_times[@]} -eq $n ]] && qjs_avg=$((qjs_sum / n))

# Always show bundled runs (rust / cranelift / llvm / wasi only exist for this single entry).
# Previously skipped under --summary-only, which hid native backends entirely.
echo ""
echo "══════════════════════════════════════════════════════════════"
echo "  BUNDLED PROGRAM OUTPUT ($entry_tish + $js_file)"
echo "  (one run per runtime; layout matches run_performance_manual.sh)"
echo "══════════════════════════════════════════════════════════════"
_show_suite_bundle_output
echo ""

pairs_list=$(mktemp)
per_row_file=$(mktemp)
./scripts/generate_perf_ci_main.sh --list-pairs >"$pairs_list"
if [[ ! -s "$pairs_list" ]]; then
  echo "Error: no test pairs from generate_perf_ci_main.sh --list-pairs" >&2
  rm -f "$pairs_list" "$per_row_file"
  exit 1
fi

micro_col_fail_vm=$(mktemp)
micro_col_fail_interp=$(mktemp)
micro_col_fail_node=$(mktemp)
micro_col_fail_bun=$(mktemp)
micro_col_fail_deno=$(mktemp)
micro_col_fail_qjs=$(mktemp)

echo ""
echo "════════════════════════════════════════════════════════════════════════════════════════════════════════════════"
echo "  PER-TEST (${n}-run avg, ms) — same workloads as tests/main.tish; sorted by vm/Node %, slowest first"
echo "  Columns match the bundled summary order: vm, interp, rust, cflt, llvm, wasi, Node, Bun, Deno, qjs."
echo "  Per-test rows: vm + interp + JS only — rust/cflt/llvm/wasi are run only in "BUNDLED PROGRAM OUTPUT" above."
echo "  Last two rows: Σ per-file = sum of each row’s ms (one micro program per row; not one runnable total)."
echo "                 bundle = same n-run averages as "BUNDLED PERF SUITE" (one tests/main.* program)."
echo "════════════════════════════════════════════════════════════════════════════════════════════════════════════════"
echo ""

total_vm=0
total_interp=0
total_node=0
total_bun=0
total_deno=0
total_qjs=0

if [ "$summary_only" = true ]; then
  echo "Running per-file micro-benchmarks (timing only; use default mode for program output per test)..."
  echo ""
else
  echo "Running per-file micro-benchmarks (each test: console output like run_performance_manual.sh, then ${n}-run avg)..."
  echo ""
fi

while IFS="$(printf '\t')" read -r tish_f js_f; do
  [[ -f "$tish_f" && -f "$js_f" ]] || continue
  tid="${tish_f#tests/}"
  tid="${tid%.tish}"
  [[ -n "${filter_name:-}" && "$tid" != *"$filter_name"* ]] && continue

  perf_tid_any_fail=0

  if [ "$summary_only" != true ]; then
    echo "─────────────────────────────────────────"
    echo "▶ $tid"
    echo "─────────────────────────────────────────"
    if want_runtime vm; then
      echo "Tish (vm):"
      _perf_run_show "micro $tid vm ($tish_f)" "$tish_bin" run "$tish_f" --backend vm
      if [[ $perf_last_subfail -eq 1 ]]; then
        perf_tid_any_fail=1
        _micro_mark_col "$micro_col_fail_vm" "$tid"
      fi
      echo ""
    fi
    if want_runtime interp; then
      echo "Tish (interp):"
      _perf_run_show "micro $tid interp ($tish_f)" "$tish_bin" run "$tish_f" --backend interp
      if [[ $perf_last_subfail -eq 1 ]]; then
        perf_tid_any_fail=1
        _micro_mark_col "$micro_col_fail_interp" "$tid"
      fi
      echo ""
    fi
    if want_runtime node; then
      echo "Node.js:"
      _perf_run_show "micro $tid node ($js_f)" "$node_cmd" "$js_f"
      if [[ $perf_last_subfail -eq 1 ]]; then
        perf_tid_any_fail=1
        _micro_mark_col "$micro_col_fail_node" "$tid"
      fi
      echo ""
    fi
    if want_runtime bun && "$has_bun"&& "$has_bun" [ "$has_bun" = true ]; then
      echo "Bun:"
      _perf_run_show "micro $tid bun ($js_f)" "$bun_cmd" "$js_f"
      if [[ $perf_last_subfail -eq 1 ]]; then
        perf_tid_any_fail=1
        _micro_mark_col "$micro_col_fail_bun" "$tid"
      fi
      echo ""
    fi
    if want_runtime deno && "$has_deno"&& "$has_deno" [ "$has_deno" = true ]; then
      echo "Deno:"
      _perf_run_show "micro $tid deno ($js_f)" "$deno_cmd" run --allow-all "$js_f"
      if [[ $perf_last_subfail -eq 1 ]]; then
        perf_tid_any_fail=1
        _micro_mark_col "$micro_col_fail_deno" "$tid"
      fi
      echo ""
    fi
    if want_runtime qjs && "$has_qjs"&& "$has_qjs" [ "$has_qjs" = true ]; then
      echo "QuickJS:"
      _perf_run_show "micro $tid qjs ($js_f)" "$qjs_cmd" "$js_f"
      if [[ $perf_last_subfail -eq 1 ]]; then
        perf_tid_any_fail=1
        _micro_mark_col "$micro_col_fail_qjs" "$tid"
      fi
      echo ""
    fi
  fi

  _pt_avg_vm "$tish_f"
  va=$_pt_avg_last_ms
  if want_runtime vm && [[ $_pt_avg_last_fail -eq 1 ]]; then
    perf_tid_any_fail=1
    _micro_mark_col "$micro_col_fail_vm" "$tid"
  fi
  _pt_avg_interp "$tish_f"
  vi=$_pt_avg_last_ms
  if want_runtime interp && [[ $_pt_avg_last_fail -eq 1 ]]; then
    perf_tid_any_fail=1
    _micro_mark_col "$micro_col_fail_interp" "$tid"
  fi
  _pt_avg_node "$js_f"
  vn=$_pt_avg_last_ms
  if want_runtime node && [[ $_pt_avg_last_fail -eq 1 ]]; then
    perf_tid_any_fail=1
    _micro_mark_col "$micro_col_fail_node" "$tid"
  fi
  _pt_avg_bun "$js_f"
  vb=$_pt_avg_last_ms
  if want_runtime bun && "$has_bun"&& "$has_bun" [ "$has_bun" = true ] && [[ $_pt_avg_last_fail -eq 1 ]]; then
    perf_tid_any_fail=1
    _micro_mark_col "$micro_col_fail_bun" "$tid"
  fi
  _pt_avg_deno "$js_f"
  vd=$_pt_avg_last_ms
  if want_runtime deno && "$has_deno"&& "$has_deno" [ "$has_deno" = true ] && [[ $_pt_avg_last_fail -eq 1 ]]; then
    perf_tid_any_fail=1
    _micro_mark_col "$micro_col_fail_deno" "$tid"
  fi
  _pt_avg_qjs "$js_f"
  vq=$_pt_avg_last_ms
  if want_runtime qjs && "$has_qjs"&& "$has_qjs" [ "$has_qjs" = true ] && [[ $_pt_avg_last_fail -eq 1 ]]; then
    perf_tid_any_fail=1
    _micro_mark_col "$micro_col_fail_qjs" "$tid"
  fi

  perf_micro_total=$((perf_micro_total + 1))
  [[ $perf_tid_any_fail -eq 1 ]] && perf_micro_failed=$((perf_micro_failed + 1))

  ratio=0
  if want_runtime vm && want_runtime node && [[ "$vn" -gt 0 ]]; then
    ratio=$((va * 100 / vn))
  fi

  printf '%d\t%s\t%d\t%d\t%d\t%d\t%d\t%d\n' "$ratio" "$tid" "$va" "$vi" "$vn" "$vb" "$vd" "$vq" >>"$per_row_file"

  total_vm=$((total_vm + va))
  total_interp=$((total_interp + vi))
  total_node=$((total_node + vn))
  total_bun=$((total_bun + vb))
  total_deno=$((total_deno + vd))
  total_qjs=$((total_qjs + vq))

  if [ "$summary_only" != true ]; then
    echo "Time (${n}-run avg, ms):"
    want_runtime vm && echo "  Tish (vm):        ${va}ms"
    want_runtime interp && echo "  Tish (interp):    ${vi}ms"
    want_runtime node && echo "  Node.js:          ${vn}ms"
    want_runtime bun && "$has_bun"&& "$has_bun" [ "$has_bun" = true ] && echo "  Bun:              ${vb}ms"
    want_runtime deno && "$has_deno"&& "$has_deno" [ "$has_deno" = true ] && echo "  Deno:             ${vd}ms"
    want_runtime qjs && "$has_qjs"&& "$has_qjs" [ "$has_qjs" = true ] && echo "  QuickJS:          ${vq}ms"
    want_runtime vm && want_runtime node && echo "  Tish(vm)/Node ratio: ${ratio}%"
    if [[ $perf_tid_any_fail -eq 1 ]]; then
      _perf_r
      echo ">>> MICRO TEST FAILED: $tid (output errors above and/or non-zero timed runs)"
      _perf_x
    fi
    echo ""
  else
    if [[ $perf_tid_any_fail -eq 1 ]]; then
      _perf_r
      printf '%s\n' "FAILED: $tid (vm: ${va}ms interp: ${vi}ms node: ${vn}ms ratio: ${ratio}%)"
      _perf_x
    else
      echo "Running $tid... done (vm: ${va}ms interp: ${vi}ms node: ${vn}ms ratio: ${ratio}%)"
    fi
  fi
done <"$pairs_list"

printf "%-24s" "Test"
want_runtime vm && printf "%7s" "vm"
want_runtime interp && printf "%7s" "interp"
want_runtime rust && printf "%7s" "rust"
want_runtime cranelift && printf "%7s" "cflt"
want_runtime llvm && printf "%7s" "llvm"
want_runtime wasi && printf "%7s" "wasi"
want_runtime node && printf "%7s" "Node"
want_runtime bun && "$has_bun"&& "$has_bun" [ "$has_bun" = true ] && printf "%7s" "Bun"
want_runtime deno && "$has_deno"&& "$has_deno" [ "$has_deno" = true ] && printf "%7s" "Deno"
want_runtime qjs && "$has_qjs"&& "$has_qjs" [ "$has_qjs" = true ] && printf "%7s" "qjs"
want_runtime vm && want_runtime node && printf "%9s" "vm/Node%"
printf "\n"

printf "%-24s" "────────────────────────"
want_runtime vm && printf "%7s" "──────"
want_runtime interp && printf "%7s" "──────"
want_runtime rust && printf "%7s" "──────"
want_runtime cranelift && printf "%7s" "──────"
want_runtime llvm && printf "%7s" "──────"
want_runtime wasi && printf "%7s" "──────"
want_runtime node && printf "%7s" "──────"
want_runtime bun && "$has_bun"&& "$has_bun" [ "$has_bun" = true ] && printf "%7s" "──────"
want_runtime deno && "$has_deno"&& "$has_deno" [ "$has_deno" = true ] && printf "%7s" "──────"
want_runtime qjs && "$has_qjs"&& "$has_qjs" [ "$has_qjs" = true ] && printf "%7s" "──────"
want_runtime vm && want_runtime node && printf "%9s" "─────────"
printf "\n"

sort -t "$(printf '\t')" -k1,1nr -k2,2 "$per_row_file" | while IFS="$(printf '\t')" read -r _r tid va vi vn vb vd vq; do
  f_vm=0
  f_interp=0
  f_node=0
  f_bun=0
  f_deno=0
  f_qjs=0
  f_pct=0
  want_runtime vm && _micro_col_has "$tid" "$micro_col_fail_vm" && f_vm=1
  want_runtime interp && _micro_col_has "$tid" "$micro_col_fail_interp" && f_interp=1
  want_runtime node && _micro_col_has "$tid" "$micro_col_fail_node" && f_node=1
  want_runtime bun && "$has_bun"&& "$has_bun" [ "$has_bun" = true ] && _micro_col_has "$tid" "$micro_col_fail_bun" && f_bun=1
  want_runtime deno && "$has_deno"&& "$has_deno" [ "$has_deno" = true ] && _micro_col_has "$tid" "$micro_col_fail_deno" && f_deno=1
  want_runtime qjs && "$has_qjs"&& "$has_qjs" [ "$has_qjs" = true ] && _micro_col_has "$tid" "$micro_col_fail_qjs" && f_qjs=1
  if want_runtime vm && want_runtime node && { [[ $f_vm -eq 1 ]] || [[ $f_node -eq 1 ]]; }; then
    f_pct=1
  fi
  printf "%-24s" "$tid"
  want_runtime vm && _perf_tbl_int7 "$f_vm" "$va"
  want_runtime interp && _perf_tbl_int7 "$f_interp" "$vi"
  want_runtime rust && printf "%7s" "—"
  want_runtime cranelift && printf "%7s" "—"
  want_runtime llvm && printf "%7s" "—"
  want_runtime wasi && printf "%7s" "—"
  want_runtime node && _perf_tbl_int7 "$f_node" "$vn"
  want_runtime bun && "$has_bun"&& "$has_bun" [ "$has_bun" = true ] && _perf_tbl_int7 "$f_bun" "$vb"
  want_runtime deno && "$has_deno"&& "$has_deno" [ "$has_deno" = true ] && _perf_tbl_int7 "$f_deno" "$vd"
  want_runtime qjs && "$has_qjs"&& "$has_qjs" [ "$has_qjs" = true ] && _perf_tbl_int7 "$f_qjs" "$vq"
  if want_runtime vm && want_runtime node && [[ "$vn" -gt 0 ]]; then
    if [[ $f_pct -eq 1 ]]; then
      _perf_r
      printf "%8d%%" "$((va * 100 / vn))"
      _perf_x
    else
      printf "%8d%%" "$((va * 100 / vn))"
    fi
  elif want_runtime vm && want_runtime node; then
    printf "%9s" "-"
  fi
  printf "\n"
done

sig_vm=0
sig_interp=0
sig_node=0
sig_bun=0
sig_deno=0
sig_qjs=0
sig_pct=0
want_runtime vm && [[ -s "$micro_col_fail_vm" ]] && sig_vm=1
want_runtime interp && [[ -s "$micro_col_fail_interp" ]] && sig_interp=1
want_runtime node && [[ -s "$micro_col_fail_node" ]] && sig_node=1
want_runtime bun && "$has_bun"&& "$has_bun" [ "$has_bun" = true ] && [[ -s "$micro_col_fail_bun" ]] && sig_bun=1
want_runtime deno && "$has_deno"&& "$has_deno" [ "$has_deno" = true ] && [[ -s "$micro_col_fail_deno" ]] && sig_deno=1
want_runtime qjs && "$has_qjs"&& "$has_qjs" [ "$has_qjs" = true ] && [[ -s "$micro_col_fail_qjs" ]] && sig_qjs=1
if want_runtime vm && want_runtime node && { [[ $sig_vm -eq 1 ]] || [[ $sig_node -eq 1 ]]; }; then
  sig_pct=1
fi

printf "%-24s" "Σ per-file (sum)"
want_runtime vm && _perf_tbl_int7 "$sig_vm" "$total_vm"
want_runtime interp && _perf_tbl_int7 "$sig_interp" "$total_interp"
want_runtime rust && printf "%7s" "—"
want_runtime cranelift && printf "%7s" "—"
want_runtime llvm && printf "%7s" "—"
want_runtime wasi && printf "%7s" "—"
want_runtime node && _perf_tbl_int7 "$sig_node" "$total_node"
want_runtime bun && "$has_bun"&& "$has_bun" [ "$has_bun" = true ] && _perf_tbl_int7 "$sig_bun" "$total_bun"
want_runtime deno && "$has_deno"&& "$has_deno" [ "$has_deno" = true ] && _perf_tbl_int7 "$sig_deno" "$total_deno"
want_runtime qjs && "$has_qjs"&& "$has_qjs" [ "$has_qjs" = true ] && _perf_tbl_int7 "$sig_qjs" "$total_qjs"
tot_ratio=0
if want_runtime vm && want_runtime node && [[ "$total_node" -gt 0 ]]; then
  tot_ratio=$((total_vm * 100 / total_node))
  if [[ $sig_pct -eq 1 ]]; then
    _perf_r
    printf "%8d%%" "$tot_ratio"
    _perf_x
  else
    printf "%8d%%" "$tot_ratio"
  fi
elif want_runtime vm && want_runtime node; then
  printf "%9s" "-"
fi
printf "\n"

printf "%-24s" "bundle (full program)"
want_runtime vm && printf "%7d" "$tish_vm_avg"
want_runtime interp && printf "%7d" "$tish_interp_avg"
want_runtime rust && if $compile_ok; then printf "%7d" "$tish_native_avg"; else printf "%7s" "—"; fi
want_runtime cranelift && if [ "$cranelift_ok" = true ]; then printf "%7d" "$tish_cranelift_avg"; else printf "%7s" "—"; fi
want_runtime llvm && if [ "$llvm_ok" = true ]; then printf "%7d" "$tish_llvm_avg"; else printf "%7s" "—"; fi
want_runtime wasi && if [ "$wasi_ok" = true ]; then printf "%7d" "$tish_wasi_avg"; else printf "%7s" "—"; fi
want_runtime node && printf "%7d" "$node_avg"
want_runtime bun && "$has_bun"&& "$has_bun" [ "$has_bun" = true ] && [[ ${#bun_times[@]} -eq $n ]] && printf "%7d" "$bun_avg"
want_runtime bun && "$has_bun"&& "$has_bun" [ "$has_bun" = true ] && [[ ${#bun_times[@]} -ne $n ]] && printf "%7s" "—"
want_runtime deno && "$has_deno"&& "$has_deno" [ "$has_deno" = true ] && [[ ${#deno_times[@]} -eq $n ]] && printf "%7d" "$deno_avg"
want_runtime deno && "$has_deno"&& "$has_deno" [ "$has_deno" = true ] && [[ ${#deno_times[@]} -ne $n ]] && printf "%7s" "—"
want_runtime qjs && "$has_qjs"&& "$has_qjs" [ "$has_qjs" = true ] && [[ ${#qjs_times[@]} -eq $n ]] && printf "%7d" "$qjs_avg"
want_runtime qjs && "$has_qjs"&& "$has_qjs" [ "$has_qjs" = true ] && [[ ${#qjs_times[@]} -ne $n ]] && printf "%7s" "—"
if want_runtime vm && want_runtime node && [[ "$node_avg" -gt 0 ]]; then
  printf "%8d%%" "$((tish_vm_avg * 100 / node_avg))"
elif want_runtime vm && want_runtime node; then
  printf "%9s" "-"
fi
printf "\n"
echo ""

echo "─────────────────────────────────────────"
echo "  Micro-test failures by column (which tid failed vm / interp / …)"
echo "─────────────────────────────────────────"
_micro_fail_any=0
if want_runtime vm && [[ -s "$micro_col_fail_vm" ]]; then
  _micro_fail_any=1
  _perf_r
  printf '%s\n' "  vm — $(sort -u "$micro_col_fail_vm" | wc -l | tr -d ' ') tid(s):"
  _perf_x
  sort -u "$micro_col_fail_vm" | sed 's/^/    /'
fi
if want_runtime interp && [[ -s "$micro_col_fail_interp" ]]; then
  _micro_fail_any=1
  _perf_r
  printf '%s\n' "  interp — $(sort -u "$micro_col_fail_interp" | wc -l | tr -d ' ') tid(s):"
  _perf_x
  sort -u "$micro_col_fail_interp" | sed 's/^/    /'
fi
if want_runtime node && [[ -s "$micro_col_fail_node" ]]; then
  _micro_fail_any=1
  _perf_r
  printf '%s\n' "  Node — $(sort -u "$micro_col_fail_node" | wc -l | tr -d ' ') tid(s):"
  _perf_x
  sort -u "$micro_col_fail_node" | sed 's/^/    /'
fi
if want_runtime bun && "$has_bun"&& "$has_bun" [ "$has_bun" = true ] && [[ -s "$micro_col_fail_bun" ]]; then
  _micro_fail_any=1
  _perf_r
  printf '%s\n' "  Bun — $(sort -u "$micro_col_fail_bun" | wc -l | tr -d ' ') tid(s):"
  _perf_x
  sort -u "$micro_col_fail_bun" | sed 's/^/    /'
fi
if want_runtime deno && "$has_deno"&& "$has_deno" [ "$has_deno" = true ] && [[ -s "$micro_col_fail_deno" ]]; then
  _micro_fail_any=1
  _perf_r
  printf '%s\n' "  Deno — $(sort -u "$micro_col_fail_deno" | wc -l | tr -d ' ') tid(s):"
  _perf_x
  sort -u "$micro_col_fail_deno" | sed 's/^/    /'
fi
if want_runtime qjs && "$has_qjs"&& "$has_qjs" [ "$has_qjs" = true ] && [[ -s "$micro_col_fail_qjs" ]]; then
  _micro_fail_any=1
  _perf_r
  printf '%s\n' "  qjs — $(sort -u "$micro_col_fail_qjs" | wc -l | tr -d ' ') tid(s):"
  _perf_x
  sort -u "$micro_col_fail_qjs" | sed 's/^/    /'
fi
if [[ $_micro_fail_any -eq 0 ]]; then
  echo "  (no per-column micro failures)"
fi
echo "─────────────────────────────────────────"
echo ""

if [ "$github_step_summary" = true ] && [[ -n "${GITHUB_STEP_SUMMARY:-}" ]]; then
  {
    echo "### Per-test (${n}-run avg, ms)"
    echo ""
    echo "| Test | vm | interp | rust | cflt | llvm | wasi | Node | Bun | Deno | qjs | vm/Node% |"
    echo "|------|---:|-------:|-----:|-----:|-----:|-----:|-----:|----:|-----:|----:|---------:|"
    sort -t "$(printf '\t')" -k1,1nr -k2,2 "$per_row_file" | while IFS="$(printf '\t')" read -r _r tid va vi vn vb vd vq; do
      pct="-"
      if want_runtime vm && want_runtime node && [[ "$vn" -gt 0 ]]; then
        pct="$((va * 100 / vn))%"
      fi
      echo "| \`${tid}\` | ${va} | ${vi} | — | — | — | — | ${vn} | ${vb} | ${vd} | ${vq} | ${pct} |"
    done
    tpct="-"
    if want_runtime vm && want_runtime node && [[ "$total_node" -gt 0 ]]; then
      tpct="$((total_vm * 100 / total_node))%"
    fi
    echo "| **Σ per-file (sum)** | **${total_vm}** | **${total_interp}** | — | — | — | — | **${total_node}** | **${total_bun}** | **${total_deno}** | **${total_qjs}** | **${tpct}** |"
    br_rust="—"
    $compile_ok && br_rust="$tish_native_avg"
    br_cflt="—"
    $cranelift_ok && br_cflt="$tish_cranelift_avg"
    br_llvm="—"
    $llvm_ok && br_llvm="$tish_llvm_avg"
    br_wasi="—"
    $wasi_ok && br_wasi="$tish_wasi_avg"
    br_bun="—"
    want_runtime bun && "$has_bun"&& "$has_bun" [ "$has_bun" = true ] && [[ ${#bun_times[@]} -eq $n ]] && br_bun="$bun_avg"
    br_deno="—"
    want_runtime deno && "$has_deno"&& "$has_deno" [ "$has_deno" = true ] && [[ ${#deno_times[@]} -eq $n ]] && br_deno="$deno_avg"
    br_qjs="—"
    want_runtime qjs && "$has_qjs"&& "$has_qjs" [ "$has_qjs" = true ] && [[ ${#qjs_times[@]} -eq $n ]] && br_qjs="$qjs_avg"
    bpct="-"
    want_runtime vm && want_runtime node && [[ "$node_avg" -gt 0 ]] && bpct="$((tish_vm_avg * 100 / node_avg))%"
    echo "| **bundle (full program)** | **${tish_vm_avg}** | **${tish_interp_avg}** | **${br_rust}** | **${br_cflt}** | **${br_llvm}** | **${br_wasi}** | **${node_avg}** | **${br_bun}** | **${br_deno}** | **${br_qjs}** | **${bpct}** |"
    echo ""
  } >>"$GITHUB_STEP_SUMMARY"
fi

rm -f "$pairs_list" "$per_row_file" \
  "$micro_col_fail_vm" "$micro_col_fail_interp" "$micro_col_fail_node" \
  "$micro_col_fail_bun" "$micro_col_fail_deno" "$micro_col_fail_qjs"

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
want_runtime bun && "$has_bun"&& "$has_bun" [ "$has_bun" = true ] && [[ ${#bun_times[@]} -eq $n ]] && echo "  Bun:              ${bun_avg}ms"
want_runtime deno && "$has_deno"&& "$has_deno" [ "$has_deno" = true ] && [[ ${#deno_times[@]} -eq $n ]] && echo "  Deno:             ${deno_avg}ms"
want_runtime qjs && "$has_qjs"&& "$has_qjs" [ "$has_qjs" = true ] && [[ ${#qjs_times[@]} -eq $n ]] && echo "  QuickJS:          ${qjs_avg}ms"
want_runtime vm && want_runtime node && echo "  Tish(vm)/Node:    ${ratio}%"
echo ""

perf_bundle_suite_fail=0
[[ $perf_bundle_failed -gt 0 || $perf_bundle_timing_fail -gt 0 ]] && perf_bundle_suite_fail=1
perf_suite_total=$((perf_micro_total + 1))
perf_suite_failed=$((perf_micro_failed + perf_bundle_suite_fail))
perf_suite_pct=0
[[ $perf_suite_total -gt 0 ]] && perf_suite_pct=$((perf_suite_failed * 100 / perf_suite_total))

echo "─────────────────────────────────────────"
if [[ $perf_suite_failed -eq 0 ]]; then
  _perf_g
  echo "PERF SUITE SUMMARY: all OK — micro-tests 0 failed of ${perf_micro_total}; bundle OK (display ${perf_bundle_failed}/${perf_bundle_checks}, timed invocations OK)"
  _perf_x
else
  _perf_r
  echo "PERF SUITE SUMMARY: ${perf_suite_failed} / ${perf_suite_total} failed (${perf_suite_pct}%)"
  _perf_x
  echo "  Micro-tests failed: ${perf_micro_failed} / ${perf_micro_total}"
  echo "  Bundle: display failures ${perf_bundle_failed} / ${perf_bundle_checks}, bundle timed-run failures: ${perf_bundle_timing_fail}"
fi
echo "─────────────────────────────────────────"
echo ""

if [ "$github_step_summary" = true ] && [[ -n "${GITHUB_STEP_SUMMARY:-}" ]]; then
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
    want_runtime bun && "$has_bun"&& "$has_bun" [ "$has_bun" = true ] && [[ ${#bun_times[@]} -eq $n ]] && echo "| Bun | ${bun_avg} |"
    want_runtime deno && "$has_deno"&& "$has_deno" [ "$has_deno" = true ] && [[ ${#deno_times[@]} -eq $n ]] && echo "| Deno | ${deno_avg} |"
    want_runtime qjs && "$has_qjs"&& "$has_qjs" [ "$has_qjs" = true ] && [[ ${#qjs_times[@]} -eq $n ]] && echo "| QuickJS | ${qjs_avg} |"
    want_runtime vm && want_runtime node && echo "| **Tish(vm)/Node %** | **${ratio}** |"
    echo ""
    echo "Profile: \`${profile}\`. ${n} timed runs after warmup; timeout ${run_timeout}s per invocation."
  } >> "$GITHUB_STEP_SUMMARY"
fi

if [[ "${PERF_SUITE_STRICT:-}" == "1" ]]; then
  if want_runtime rust && ! $compile_ok; then
    echo "ERROR: PERF_SUITE_STRICT=1 but rust native binary missing or failed to build"
    if [[ -s "$compile_dir/last_rust_native_build.log" ]]; then
      echo "  (saved rust native cargo log — last 80 lines:)"
      tail -80 "$compile_dir/last_rust_native_build.log" | sed 's/^/  | /'
    fi
    if [[ -e "$native_bin" ]]; then
      echo "  (native output path exists but is not executable — ls -la:)"
      ls -la "$native_bin" # codacy-disable-line 2>&1 | sed 's/^/  | /' || true
    fi
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
  if want_runtime wasi && "$has_wasmtime"&& "$has_wasmtime" [ "$has_wasmtime" = true ] && ! "$wasi_ok"&& ! "$wasi_ok" [ "$wasi_ok" != true ]; then
    echo "ERROR: PERF_SUITE_STRICT=1 but wasi build failed or wasm missing"
    exit 1
  fi
fi

echo "Done."
