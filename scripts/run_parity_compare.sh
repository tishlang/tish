#!/bin/bash
# Compare stdout of tests/core programs across runtimes. Fails if any runtime
# output differs from the reference. Use to find VM/Cranelift/WASI/Node parity gaps.
#
# By default, JS runtimes (node, bun, deno) output is normalized: "undefined" -> "null"
# for comparison, since Tish uses null where JavaScript uses undefined.
#
# Usage: ./scripts/run_parity_compare.sh [OPTIONS]
#
# Options:
#   --reference REF   Reference runtime: interp (default), node, rust, cranelift, llvm
#   --runtimes R,...   Comma-separated: interp,vm,rust,cranelift,llvm,wasi,node,bun,deno (default: all available)
#   --filter NAME      Run only tests matching NAME (e.g. optional_chaining)
#   --limit N         Run only first N tests
#   --no-compile      Use cached binaries; skip compile step (run without --no-compile first)
#   --timeout SEC     Timeout per run in seconds (default: 15, 0 = no limit)
#   --verbose         Show reference and runtime output on mismatch
#
# Example:
#   ./scripts/run_parity_compare.sh --filter optional_chaining
#   ./scripts/run_parity_compare.sh --runtimes interp,vm,node --limit 5
#
# Exit: 0 if all runtimes match reference; 1 if any mismatch or run failure.

set -e
cd "$(dirname "$0")/.."

core_dir="tests/core"
target_dir="$(pwd)/target"
profile="debug"
reference="interp"
runtimes_filter=""
filter_name=""
limit=0
no_compile=false
run_timeout=15
verbose=false

# Parse arguments
while [[ $# -gt 0 ]]; do
  case "$1" in
    --reference) reference="$2"; shift 2 ;;
    --runtimes) runtimes_filter="$2"; shift 2 ;;
    --filter) filter_name="$2"; shift 2 ;;
    --limit) limit="$2"; shift 2 ;;
    --no-compile) no_compile=true; shift ;;
    --timeout) run_timeout="$2"; shift 2 ;;
    --verbose) verbose=true; shift ;;
    *) echo "Unknown option: $1"; exit 1 ;;
  esac
done

want_runtime() {
  local r="$1"
  if [[ -z "$runtimes_filter" ]]; then
    return 0
  fi
  [[ ",${runtimes_filter}," == *",${r},"* ]]
}

# Runtime detection
node_cmd="${NODE:-node}"
bun_cmd="${BUN:-bun}"
deno_cmd="${DENO:-deno}"
has_bun=false; has_deno=false; has_wasmtime=false
command -v "$bun_cmd" &>/dev/null && has_bun=true
command -v "$deno_cmd" &>/dev/null && has_deno=true
command -v wasmtime &>/dev/null && has_wasmtime=true

tish_bin="$target_dir/$profile/tish"
cache_dir="$target_dir/parity-cache-$profile"
mkdir -p "$cache_dir"

# Timeout wrapper: stop process after run_timeout seconds. Uses timeout/gtimeout, perl, or bash.
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
        my $t=shift; my $pid=fork; die "fork: $!" if !defined $pid;
        if ($pid==0) { setpgrp(0,0) if defined &setpgrp; exec(@ARGV) or exit 127; }
        $SIG{ALRM}=sub{ kill 9,-$pid; kill 9,$pid; waitpid $pid,0; exit 124 };
        alarm $t; waitpid $pid,0; alarm 0; exit($?>>8)
      ' "$run_timeout" "$@" || true
    else
      perl -e '
        my $t=shift; my $pid=fork; die "fork: $!" if !defined $pid;
        if ($pid==0) { setpgrp(0,0) if defined &setpgrp; exec(@ARGV) or exit 127; }
        $SIG{ALRM}=sub{ kill 9,-$pid; kill 9,$pid; waitpid $pid,0; exit 124 };
        alarm $t; waitpid $pid,0; alarm 0; exit($?>>8)
      ' "$run_timeout" "$@" 2>/dev/null || true
    fi
    return
  fi
  ( set +e; set -m; if $verbose; then "$@" & else "$@" 2>/dev/null & fi; pid=$!; ( sleep $run_timeout; kill -TERM -$pid 2>/dev/null; sleep 2; kill -KILL -$pid 2>/dev/null ) & k=$!; wait $pid 2>/dev/null; kill $k 2>/dev/null; wait $k 2>/dev/null ) || true
}

# Capture stdout; when verbose, stderr is left visible (no 2>/dev/null)
run_and_capture() {
  if $verbose; then
    run_with_timeout "$@"
  else
    run_with_timeout "$@" 2>/dev/null
  fi
}

# Normalize JS output: undefined -> null; typeof null "object" -> "null" (Tish semantics)
normalize_js_output() {
  printf '%s' "$1" | sed 's/undefined/null/g' | sed '/^boolean$/{n;s/^object$/null/;}'
}

# Normalize timing output: " Xms" -> " 0ms" for parity (timing varies by runtime)
normalize_timing() {
  printf '%s' "$1" | sed 's/ [0-9][0-9]*ms/ 0ms/g'
}

# Build tish
if [[ ! -x "$tish_bin" ]]; then
  echo "Building tish ($profile)..."
  cargo build -p tish --target-dir "$target_dir" -q 2>/dev/null || true
  [[ ! -x "$tish_bin" ]] && tish_bin="cargo run -p tish --target-dir $target_dir -q --"
fi

echo "=== Tish Parity Compare ==="
echo "Reference: $reference"
echo "Runtimes: ${runtimes_filter:-all available}"
[[ -n "$filter_name" ]] && echo "Filter: $filter_name"
[[ $limit -gt 0 ]] && echo "Limit: $limit"
echo ""

fail_count=0
pass_count=0
skip_count=0

for f in "$core_dir"/*.tish; do
  [[ -f "$f" ]] || continue
  base=$(basename "$f" .tish)
  [[ -n "$filter_name" && "$base" != *"$filter_name"* ]] && continue
  js_file="$core_dir/$base.js"
  [[ -f "$js_file" ]] || continue

  ref_out=""
  case "$reference" in
    interp)
      ref_out=$(run_and_capture $tish_bin run "$f" --backend interp || true)
      ;;
    node)
      ref_out=$(normalize_js_output "$(run_and_capture "$node_cmd" "$js_file" || true)")
      ;;
    rust)
      rust_bin="$cache_dir/${base}_native"
      if ! $no_compile; then
        $tish_bin compile "$f" -o "$rust_bin" --native-backend rust >/dev/null 2>&1 || true
      fi
      [[ -x "$rust_bin" ]] && ref_out=$(run_and_capture "$rust_bin" || true)
      ;;
    cranelift)
      cl_bin="$cache_dir/${base}_cranelift"
      if ! $no_compile; then
        $tish_bin compile "$f" -o "$cl_bin" --native-backend cranelift >/dev/null 2>&1 || true
      fi
      [[ -x "$cl_bin" ]] && ref_out=$(run_and_capture "$cl_bin" || true)
      ;;
    llvm)
      llvm_bin="$cache_dir/${base}_llvm"
      if ! $no_compile; then
        $tish_bin compile "$f" -o "$llvm_bin" --native-backend llvm >/dev/null 2>&1 || true
      fi
      [[ -x "$llvm_bin" ]] && ref_out=$(run_and_capture "$llvm_bin" || true)
      ;;
    *) echo "Unknown reference: $reference"; exit 1 ;;
  esac

  ref_normalized=$(normalize_timing "$ref_out")

  any_fail=false
  failures=()

  # Compare: interp (unless reference)
  if want_runtime interp && [[ "$reference" != "interp" ]]; then
    out=$(normalize_timing "$(run_and_capture $tish_bin run "$f" --backend interp || true)")
    if [[ "$out" != "$ref_normalized" ]]; then
      any_fail=true
      failures+=("interp")
    fi
  fi

  # Compare: vm
  if want_runtime vm; then
    out=$(normalize_timing "$(run_and_capture $tish_bin run "$f" --backend vm || true)")
    if [[ "$out" != "$ref_normalized" ]]; then
      any_fail=true
      failures+=("vm")
    fi
  fi

  # Compare: rust
  if want_runtime rust && [[ "$reference" != "rust" ]]; then
    rust_bin="$cache_dir/${base}_native"
    if ! $no_compile; then
      $tish_bin compile "$f" -o "$rust_bin" --native-backend rust >/dev/null 2>&1 || true
    fi
    if [[ -x "$rust_bin" ]]; then
      out=$(normalize_timing "$(run_and_capture "$rust_bin" || true)")
      if [[ "$out" != "$ref_normalized" ]]; then
        any_fail=true
        failures+=("rust")
      fi
    else
      failures+=("rust(no binary)")
      any_fail=true
    fi
  fi

  # Compare: cranelift
  if want_runtime cranelift && [[ "$reference" != "cranelift" ]]; then
    cl_bin="$cache_dir/${base}_cranelift"
    if ! $no_compile; then
      $tish_bin compile "$f" -o "$cl_bin" --native-backend cranelift >/dev/null 2>&1 || true
    fi
    if [[ -x "$cl_bin" ]]; then
      out=$(normalize_timing "$(run_and_capture "$cl_bin" || true)")
      if [[ "$out" != "$ref_normalized" ]]; then
        any_fail=true
        failures+=("cranelift")
      fi
    else
      failures+=("cranelift(no binary)")
      any_fail=true
    fi
  fi

  # Compare: llvm
  if want_runtime llvm && [[ "$reference" != "llvm" ]]; then
    llvm_bin="$cache_dir/${base}_llvm"
    if ! $no_compile; then
      $tish_bin compile "$f" -o "$llvm_bin" --native-backend llvm >/dev/null 2>&1 || true
    fi
    if [[ -x "$llvm_bin" ]]; then
      out=$(normalize_timing "$(run_and_capture "$llvm_bin" || true)")
      if [[ "$out" != "$ref_normalized" ]]; then
        any_fail=true
        failures+=("llvm")
      fi
    else
      failures+=("llvm(no binary)")
      any_fail=true
    fi
  fi

  # Compare: wasi
  if want_runtime wasi && $has_wasmtime; then
    wasi_bin="$cache_dir/${base}_wasi.wasm"
    if ! $no_compile; then
      $tish_bin compile "$f" -o "$cache_dir/${base}_wasi" --target wasi >/dev/null 2>&1 || true
    fi
    if [[ -f "$wasi_bin" ]]; then
      out=$(normalize_timing "$(run_and_capture wasmtime "$wasi_bin" || true)")
      if [[ "$out" != "$ref_normalized" ]]; then
        any_fail=true
        failures+=("wasi")
      fi
    else
      failures+=("wasi(no wasm)")
      any_fail=true
    fi
  fi

  # Compare: node
  if want_runtime node && [[ "$reference" != "node" ]]; then
    out=$(normalize_timing "$(normalize_js_output "$(run_and_capture "$node_cmd" "$js_file" || true)")")
    if [[ "$out" != "$ref_normalized" ]]; then
      any_fail=true
      failures+=("node")
    fi
  fi

  # Compare: bun
  if want_runtime bun && $has_bun; then
    out=$(normalize_timing "$(normalize_js_output "$(run_and_capture "$bun_cmd" "$js_file" || true)")")
    if [[ "$out" != "$ref_normalized" ]]; then
      any_fail=true
      failures+=("bun")
    fi
  fi

  # Compare: deno
  if want_runtime deno && $has_deno; then
    out=$(normalize_timing "$(normalize_js_output "$(run_and_capture "$deno_cmd" run --allow-all "$js_file" || true)")")
    if [[ "$out" != "$ref_normalized" ]]; then
      any_fail=true
      failures+=("deno")
    fi
  fi

  if $any_fail; then
    echo "FAIL $base [${failures[*]}]"
    if $verbose; then
      echo "  Reference ($reference) stdout:"
      echo "$ref_out" | sed 's/^/    /'
      for r in "${failures[@]}"; do
        echo "  Runtime $r output (stderr shown if verbose):"
        case "$r" in
          vm) run_with_timeout $tish_bin run "$f" --backend vm | sed 's/^/    /' || true ;;
          cranelift) [[ -x "$cache_dir/${base}_cranelift" ]] && run_with_timeout "$cache_dir/${base}_cranelift" | sed 's/^/    /' || true ;;
          llvm) [[ -x "$cache_dir/${base}_llvm" ]] && run_with_timeout "$cache_dir/${base}_llvm" | sed 's/^/    /' || true ;;
          wasi) [[ -f "$cache_dir/${base}_wasi.wasm" ]] && run_with_timeout wasmtime "$cache_dir/${base}_wasi.wasm" | sed 's/^/    /' || true ;;
          node) run_with_timeout "$node_cmd" "$js_file" | sed 's/^/    /' || true ;;
          *) ;;
        esac
      done
    fi
    fail_count=$((fail_count + 1))
  else
    echo "OK   $base"
    pass_count=$((pass_count + 1))
  fi

  if [[ $limit -gt 0 ]]; then
    total=$((pass_count + fail_count))
    [[ $total -ge $limit ]] && break
  fi
done

echo ""
echo "Summary: $pass_count passed, $fail_count failed (reference=$reference)"
[[ $fail_count -gt 0 ]] && exit 1
exit 0
