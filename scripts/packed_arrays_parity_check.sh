#!/usr/bin/env bash
# TISH_PACKED_ARRAYS=1 parity sweep (#199) — the gate for ever flipping packed f64 arrays
# default-on. Two parts:
#
#   1. CORPUS: every tests/core/*.tish with a committed .expected runs FLAG-OFF and FLAG-ON on
#      the interpreter, the bytecode VM, and (unless --no-native) a native build — and the two
#      outputs must be identical per backend. The flag is read once per process via a OnceLock
#      (#239/#166), so a single native binary is built per fixture and run twice with the env
#      var toggled; no double build. Inherent nondeterminism (`<N>ms` timings) is normalized
#      before diffing, mirroring scripts/flags_on_corpus_check.sh; fixtures whose stdout is
#      nondeterministic BY DESIGN are skipped, mirroring the parity_skip list in
#      scripts/run_parity_compare.sh / TIMING_NONDETERMINISTIC in integration_test.rs.
#
#   2. PERF CHECKSUMS: every tests/perf/*.tish prints `GAUNTLET <name> <ms> <check>`; the
#      <check> column must be identical for VM flag-off, VM flag-on, and node (the fixtures are
#      valid in both languages; a `<name>.js` sibling overrides the node source). This is the
#      differential the packed VM fast paths (NewArray, HOFs, sort) must preserve.
#
# Any divergence is a release blocker for the default flip: file it as a bug, link it in #199,
# and (only if it must not block unrelated CI) add the fixture to KNOWN_DIVERGENCES below with
# the issue number.
#
# Usage: scripts/packed_arrays_parity_check.sh [--no-native] [--corpus-only|--perf-only]
# Env:   TISH_BIN (default target/release/tish)
set -u
cd "$(dirname "$0")/.." || exit 1
TISH="${TISH_BIN:-target/release/tish}"

RUN_NATIVE=1
RUN_CORPUS=1
RUN_PERF=1
for arg in "$@"; do
  case "$arg" in
    --no-native) RUN_NATIVE=0 ;;
    --corpus-only) RUN_PERF=0 ;;
    --perf-only) RUN_CORPUS=0 ;;
    *) echo "unknown arg: $arg" >&2; exit 2 ;;
  esac
done

if [ ! -x "$TISH" ]; then
  echo "tish binary not found at $TISH — build with: cargo build --release -p tishlang --bin tish" >&2
  exit 2
fi

# Fixtures whose stdout is nondeterministic or backend-specific BY DESIGN (timings, stress
# diagnostics). Mirrors run_parity_compare.sh's parity_skip / integration_test.rs's
# TIMING_NONDETERMINISTIC.
SKIP=" array_stress array_stress_01_large_array_creation array_stress_02_iteration \
array_stress_03_map_filter_reduce array_stress_04_chained array_stress_05_sorting \
array_stress_06_search array_stress_07_splice_slice array_stress_08_concat_spread \
array_stress_09_flat array_stress_10_objects basic_types benchmark_granular new_features_perf \
object_stress objects_perf string_methods_perf recursion_stress jit_probe jit_regression "

# Fixtures with a KNOWN, FILED flag-on divergence (format: "name:#issue"). A known fixture is
# reported and counted separately, not failed — but the default flip stays blocked until this
# list is empty (see #199). Filed 2026-07-17 by the first full sweep:
#   #502 fill no-op · #503 for..in empty · #504 delete leaves value · #505 unshift corruption
#   #506 splice removed-values wrong · #507 flat misses packed inners
#   (#508 native F64A.reduce — FIXED: fused-reduce NumberArray arm)
KNOWN_DIVERGENCES=""

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
norm() { sed -E 's/[0-9]+(\.[0-9]+)?ms/Nms/g'; }

known_issue_for() { # name -> issue ref or empty
  local entry
  for entry in $KNOWN_DIVERGENCES; do
    if [ "${entry%%:*}" = "$1" ]; then echo "${entry#*:}"; return; fi
  done
}

fail=0; known=0

if [ "$RUN_CORPUS" = 1 ]; then
  n=0; pass=0
  for src in tests/core/*.tish; do
    [ -f "${src}.expected" ] || continue
    base=$(basename "${src%.tish}")
    [[ "$SKIP" == *" $base "* ]] && continue
    n=$((n+1))
    diverged=""
    for backend in interp vm; do
      off=$(env -u TISH_PACKED_ARRAYS "$TISH" run --backend "$backend" "$src" 2>/dev/null | norm)
      on=$(env TISH_PACKED_ARRAYS=1 "$TISH" run --backend "$backend" "$src" 2>/dev/null | norm)
      if [ "$on" != "$off" ]; then
        diverged="$diverged $backend"
        printf 'DIFF  %-45s [%s]\n' "$base" "$backend"
        diff <(printf '%s' "$off") <(printf '%s' "$on") | head -6 | sed 's/^/    /'
      fi
    done
    if [ "$RUN_NATIVE" = 1 ]; then
      bin="$tmp/$base"
      if env TISH_FAST_NATIVE_BUILD=1 "$TISH" build "$src" -o "$bin" >/dev/null 2>"$tmp/err"; then
        off=$(env -u TISH_PACKED_ARRAYS "$bin" 2>/dev/null | norm)
        on=$(env TISH_PACKED_ARRAYS=1 "$bin" 2>/dev/null | norm)
        if [ "$on" != "$off" ]; then
          diverged="$diverged native"
          printf 'DIFF  %-45s [native]\n' "$base"
          diff <(printf '%s' "$off") <(printf '%s' "$on") | head -6 | sed 's/^/    /'
        fi
        rm -f "$bin"
      else
        printf 'BUILD-FAIL  %-39s [native]\n' "$base"
        sed 's/^/    /' "$tmp/err" | head -4
        diverged="$diverged native-build"
      fi
    fi
    if [ -z "$diverged" ]; then
      pass=$((pass+1))
    else
      issue=$(known_issue_for "$base")
      if [ -n "$issue" ]; then
        echo "KNOWN ($issue): $base —$diverged"
        known=$((known+1))
      else
        fail=$((fail+1))
      fi
    fi
  done
  echo "corpus: $pass/$n flag-on == flag-off, $known known (filed), $((n - pass - known)) new divergence(s)"
fi

if [ "$RUN_PERF" = 1 ]; then
  pn=0; ppass=0
  for src in tests/perf/*.tish; do
    base=$(basename "${src%.tish}")
    pn=$((pn+1))
    line_off=$(env -u TISH_PACKED_ARRAYS "$TISH" run --backend vm "$src" 2>/dev/null | grep "^GAUNTLET " | tail -1)
    line_on=$(env TISH_PACKED_ARRAYS=1 "$TISH" run --backend vm "$src" 2>/dev/null | grep "^GAUNTLET " | tail -1)
    js="$src"
    [ -f "tests/perf/${base}.js" ] && js="tests/perf/${base}.js"
    line_node=$(node "$js" 2>/dev/null | grep "^GAUNTLET " | tail -1)
    c_off=$(echo "$line_off" | awk '{print $4}')
    c_on=$(echo "$line_on" | awk '{print $4}')
    c_node=$(echo "$line_node" | awk '{print $4}')
    if [ -n "$c_off" ] && [ "$c_off" = "$c_on" ] && [ "$c_off" = "$c_node" ]; then
      ppass=$((ppass+1))
    else
      issue=$(known_issue_for "$base")
      if [ -n "$issue" ]; then
        echo "KNOWN ($issue): perf $base checksum off=$c_off on=$c_on node=$c_node"
        known=$((known+1))
      else
        echo "CHECKSUM-DIFF  $base: off=${c_off:-MISSING} on=${c_on:-MISSING} node=${c_node:-MISSING}"
        fail=$((fail+1))
      fi
    fi
  done
  echo "perf checksums: $ppass/$pn flag-on == flag-off == node"
fi

echo "packed-arrays parity: ${fail} new divergence(s), ${known} known (filed)"
[ "$fail" -eq 0 ]
