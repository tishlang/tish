#!/bin/bash
# Compare stdout of tests/core programs across runtimes. Fails if any runtime
# output differs from the reference. Use to find VM/Cranelift/WASI/Node parity gaps.
#
# Usage: ./scripts/run_parity_compare.sh [OPTIONS]
#
# Options:
#   --reference REF   Reference runtime: interp (default), node, rust
#   --runtimes R,...   Comma-separated: interp,vm,rust,cranelift,wasi,node,bun,deno (default: all available)
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
      ref_out=$(run_with_timeout $tish_bin run "$f" --backend interp 2>/dev/null || true)
      ;;
    node)
      ref_out=$(run_with_timeout "$node_cmd" "$js_file" 2>/dev/null || true)
      ;;
    rust)
      rust_bin="$cache_dir/${base}_native"
      if ! $no_compile; then
        $tish_bin compile "$f" -o "$rust_bin" --native-backend rust >/dev/null 2>&1 || true
      fi
      [[ -x "$rust_bin" ]] && ref_out=$(run_with_timeout "$rust_bin" 2>/dev/null || true)
      ;;
    *) echo "Unknown reference: $reference"; exit 1 ;;
  esac

  any_fail=false
  failures=()

  # Compare: interp (unless reference)
  if want_runtime interp && [[ "$reference" != "interp" ]]; then
    out=$(run_with_timeout $tish_bin run "$f" --backend interp 2>/dev/null || true)
    if [[ "$out" != "$ref_out" ]]; then
      any_fail=true
      failures+=("interp")
    fi
  fi

  # Compare: vm
  if want_runtime vm; then
    out=$(run_with_timeout $tish_bin run "$f" --backend vm 2>/dev/null || true)
    if [[ "$out" != "$ref_out" ]]; then
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
      out=$(run_with_timeout "$rust_bin" 2>/dev/null || true)
      if [[ "$out" != "$ref_out" ]]; then
        any_fail=true
        failures+=("rust")
      fi
    else
      failures+=("rust(no binary)")
      any_fail=true
    fi
  fi

  # Compare: cranelift
  if want_runtime cranelift; then
    cl_bin="$cache_dir/${base}_cranelift"
    if ! $no_compile; then
      $tish_bin compile "$f" -o "$cl_bin" --native-backend cranelift >/dev/null 2>&1 || true
    fi
    if [[ -x "$cl_bin" ]]; then
      out=$(run_with_timeout "$cl_bin" 2>/dev/null || true)
      if [[ "$out" != "$ref_out" ]]; then
        any_fail=true
        failures+=("cranelift")
      fi
    else
      failures+=("cranelift(no binary)")
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
      out=$(run_with_timeout wasmtime "$wasi_bin" 2>/dev/null || true)
      if [[ "$out" != "$ref_out" ]]; then
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
    out=$(run_with_timeout "$node_cmd" "$js_file" 2>/dev/null || true)
    if [[ "$out" != "$ref_out" ]]; then
      any_fail=true
      failures+=("node")
    fi
  fi

  # Compare: bun
  if want_runtime bun && $has_bun; then
    out=$(run_with_timeout "$bun_cmd" "$js_file" 2>/dev/null || true)
    if [[ "$out" != "$ref_out" ]]; then
      any_fail=true
      failures+=("bun")
    fi
  fi

  # Compare: deno
  if want_runtime deno && $has_deno; then
    out=$(run_with_timeout "$deno_cmd" run --allow-all "$js_file" 2>/dev/null || true)
    if [[ "$out" != "$ref_out" ]]; then
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
        echo "  Runtime $r output:"
        case "$r" in
          vm) run_with_timeout $tish_bin run "$f" --backend vm 2>/dev/null | sed 's/^/    /' || true ;;
          cranelift) [[ -x "$cache_dir/${base}_cranelift" ]] && run_with_timeout "$cache_dir/${base}_cranelift" 2>/dev/null | sed 's/^/    /' || true ;;
          wasi) [[ -f "$cache_dir/${base}_wasi.wasm" ]] && run_with_timeout wasmtime "$cache_dir/${base}_wasi.wasm" 2>/dev/null | sed 's/^/    /' || true ;;
          node) run_with_timeout "$node_cmd" "$js_file" 2>/dev/null | sed 's/^/    /' || true ;;
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
