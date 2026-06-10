#!/usr/bin/env bash
# PERF GAUNTLET — compute-only benchmarks (self-timed; process startup excluded) that validate the
# typed-native codegen by building each fixture TWICE on the rust backend:
#
#   boxed(off) = all typing flags OFF  → the dynamic `Value` path (the untyped baseline)
#   typed(on)  = all typing flags ON   → native f64 / Vec<f64> / String / structs (the typing work)
#
# and timing both against node (V8). Columns:
#   typing-speedup = boxed(off) / typed(on)   — the win attributable to the typing work (A/B).
#   node           = typed(on) time vs V8, with the ratio.
#   status         PASS = typed(on) <= node AND typed == boxed == node result;
#                  FAIL = typed(on) slower than V8 (a known gap to evolve past);
#                  TYPED≠BOXED = the typed path changed the result (a typing bug — investigate!);
#                  ≠NODE = a backend result disagrees with V8.
#
# Benchmarks live in tests/perf/<name>.tish; if <name>.js exists node runs that, else it runs the
# .tish directly (those files are written to be valid in both tish and node).
#
#   scripts/run_perf_gauntlet.sh [--runs N] [--no-build] [name ...]
set -uo pipefail
cd "$(dirname "$0")/.."
command -v node >/dev/null 2>&1 || { echo "missing node"; exit 1; }
TISH="${TISH_BIN:-target/release/tish}"
[[ -x "$TISH" ]] || { echo "no tish at $TISH (cargo build -p tishlang --release)"; exit 1; }

# Every dark-shipped typed-native flag — keep this in lockstep with docs/type-system-roadmap.md.
TYPED_FLAGS=(
  TISH_PARAM_NATIVE=1   # M1 annotated scalar params
  TISH_PARAM_INFER=1    # M4 numeric param inference
  TISH_NATIVE_FN=1      # M5 native monomorphic fns
  TISH_STRUCT_INFER=1   # struct / array-literal inference
  TISH_FUSED_HOF=1      # fused reduce over a boxed array
  TISH_NATIVE_HOF=1     # native reduce/map/filter/some/every over a `number[]` (Vec<f64>)
)

RUNS=3; NO_BUILD=0; ONLY=()
while [[ $# -gt 0 ]]; do
  case "$1" in
    --runs) RUNS="$2"; shift 2 ;;
    --no-build) NO_BUILD=1; shift ;;
    *) ONLY+=("$1"); shift ;;
  esac
done

# Echo "min_ms check" over RUNS executions of a cmd printing "GAUNTLET <name> <ms> <check>".
run_min() {
  local name="$1"; shift
  local best="" check="" r line ms ck
  for ((r = 0; r < RUNS; r++)); do
    line=$("$@" 2>/dev/null | grep "^GAUNTLET ${name} " | head -1)
    if [[ -z "$line" ]]; then echo "ERR ERR"; return; fi
    ms=$(printf '%s\n' "$line" | awk '{print $3}')
    ck=$(printf '%s\n' "$line" | awk '{print $4}')
    check="$ck"
    if [[ -z "$best" || "$ms" -lt "$best" ]]; then best="$ms"; fi
  done
  echo "$best $check"
}

printf 'PERF GAUNTLET — typed-native A/B: boxed(flags-off) vs typed(flags-on) vs node V8, min of %d runs\n' "$RUNS"
printf 'typing-speedup = boxed/typed (the win from the typing work);  PASS = typed <= node AND typed == boxed.\n\n'

rows=(); pass=0; total=0
for tish_src in tests/perf/*.tish; do
  name=$(basename "$tish_src" .tish)
  if [[ ${#ONLY[@]} -gt 0 ]] && ! printf '%s\n' "${ONLY[@]}" | grep -qx "$name"; then continue; fi
  bin_on="/tmp/gauntlet_${name}_typed"
  bin_off="/tmp/gauntlet_${name}_boxed"
  if [[ "$NO_BUILD" -eq 0 ]]; then
    # typed(on): all typed-native flags set.
    if ! env "${TYPED_FLAGS[@]}" "$TISH" build "$tish_src" -o "$bin_on" \
        --target native --native-backend rust >/dev/null 2>&1; then
      rows+=("${name}|-|BUILD-FAIL|-|-|-"); total=$((total + 1)); continue
    fi
    # boxed(off): same source + backend, every typing flag unset (the dynamic Value baseline).
    if ! env -u TISH_PARAM_NATIVE -u TISH_PARAM_INFER -u TISH_NATIVE_FN -u TISH_STRUCT_INFER \
            -u TISH_FUSED_HOF -u TISH_NATIVE_HOF "$TISH" build "$tish_src" -o "$bin_off" \
        --target native --native-backend rust >/dev/null 2>&1; then
      rows+=("${name}|BUILD-FAIL|-|-|-|-"); total=$((total + 1)); continue
    fi
  fi
  read -r on_ms on_ck < <(run_min "$name" "$bin_on")
  read -r off_ms off_ck < <(run_min "$name" "$bin_off")
  node_src="tests/perf/${name}.js"; [[ -f "$node_src" ]] || node_src="$tish_src"
  read -r node_ms node_ck < <(run_min "$name" node "$node_src")
  total=$((total + 1))
  if [[ "$on_ms" == "ERR" || "$off_ms" == "ERR" || "$node_ms" == "ERR" ]]; then
    rows+=("${name}|${off_ms}|${on_ms}|-|${node_ms}|RUN-ERR")
  elif [[ "$off_ck" != "$on_ck" ]]; then
    # The typing flags changed the computed result — a soundness regression, not a perf one.
    rows+=("${name}|${off_ms}ms|${on_ms}ms|-|${node_ms}ms|TYPED≠BOXED")
  elif [[ "$on_ck" != "$node_ck" ]]; then
    rows+=("${name}|${off_ms}ms|${on_ms}ms|-|${node_ms}ms|≠NODE")
  else
    speedup=$(awk "BEGIN{printf \"%.2f\", ${off_ms}/(${on_ms}+0.001)}")
    ratio=$(awk "BEGIN{printf \"%.2f\", ${on_ms}/(${node_ms}+0.001)}")
    if awk "BEGIN{exit !(${on_ms} <= ${node_ms})}"; then
      verdict="PASS ✓"; pass=$((pass + 1))
    else
      verdict="FAIL ✗ (evolve)"
    fi
    rows+=("${name}|${off_ms}ms|${on_ms}ms|${speedup}x|${node_ms}ms (${ratio}x)|${verdict}")
  fi
done

{
  printf 'benchmark|boxed(off)|typed(on)|typing-speedup|node(ratio)|status\n'
  for r in "${rows[@]}"; do printf '%s\n' "$r"; done
} | column -t -s '|'
echo ""
echo "SUMMARY: ${pass}/${total} typed-native beating V8."
echo "  typing-speedup = boxed(flags-off) / typed(flags-on) — the speedup the typing work delivers."
