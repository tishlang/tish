#!/usr/bin/env bash
# PERF GAUNTLET — compute-only benchmarks (self-timed; process startup excluded) for the rust
# backend vs node (V8). DELIBERATELY includes targets we currently LOSE, so each backend
# improvement can be measured and we can watch red turn green over time.
#
#   PASS = tish <= node on the hot-loop time AND identical result.
#   FAIL = a known gap to evolve past (the whole point of this corpus).
#
# Benchmarks live in tests/perf/<name>.tish; if <name>.js exists node runs that, else it runs
# the .tish directly (those files are written to be valid in both tish and node).
#
#   scripts/run_perf_gauntlet.sh [--runs N] [--no-build] [name ...]
set -uo pipefail
cd "$(dirname "$0")/.."
command -v node >/dev/null 2>&1 || { echo "missing node"; exit 1; }
TISH="${TISH_BIN:-target/release/tish}"
[[ -x "$TISH" ]] || { echo "no tish at $TISH (cargo build -p tishlang --release)"; exit 1; }

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

printf 'PERF GAUNTLET — tish rust-AOT (TISH_PARAM_NATIVE=1) vs node V8 — compute-only, min of %d runs\n' "$RUNS"
printf 'PASS = tish <= node AND same result;  FAIL = a known gap to evolve past.\n\n'

rows=(); pass=0; total=0
for tish_src in tests/perf/*.tish; do
  name=$(basename "$tish_src" .tish)
  if [[ ${#ONLY[@]} -gt 0 ]] && ! printf '%s\n' "${ONLY[@]}" | grep -qx "$name"; then continue; fi
  bin="/tmp/gauntlet_${name}"
  if [[ "$NO_BUILD" -eq 0 ]]; then
    if ! TISH_PARAM_NATIVE=1 "$TISH" build "$tish_src" -o "$bin" \
        --target native --native-backend rust >/dev/null 2>&1; then
      rows+=("${name}|BUILD-FAIL|-|-|-"); total=$((total + 1)); continue
    fi
  fi
  read -r tish_ms tish_ck < <(run_min "$name" "$bin")
  node_src="tests/perf/${name}.js"; [[ -f "$node_src" ]] || node_src="$tish_src"
  read -r node_ms node_ck < <(run_min "$name" node "$node_src")
  total=$((total + 1))
  if [[ "$tish_ms" == "ERR" || "$node_ms" == "ERR" ]]; then
    rows+=("${name}|${tish_ms}|${node_ms}|-|RUN-ERR")
  elif [[ "$tish_ck" != "$node_ck" ]]; then
    rows+=("${name}|${tish_ms}ms|${node_ms}ms|-|WRONG")
  else
    ratio=$(awk "BEGIN{printf \"%.2f\", ${tish_ms}/(${node_ms}+0.001)}")
    if awk "BEGIN{exit !(${tish_ms} <= ${node_ms})}"; then
      verdict="PASS ✓"; pass=$((pass + 1))
    else
      verdict="FAIL ✗  (evolve)"
    fi
    rows+=("${name}|${tish_ms}ms|${node_ms}ms|${ratio}x|${verdict}")
  fi
done

{
  printf 'benchmark|tish|node|ratio|status\n'
  for r in "${rows[@]}"; do printf '%s\n' "$r"; done
} | column -t -s '|'
echo ""
echo "SUMMARY: ${pass}/${total} beating V8 — $((total - pass)) targets to evolve toward."
