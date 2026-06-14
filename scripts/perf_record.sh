#!/usr/bin/env bash
# Record a bundled-perf-suite run as a structured TSV under perf-history/, so performance can be
# tracked OVER TIME (per commit) instead of measured once and thrown away. Companion to
# scripts/perf_compare.sh, which diffs two records.
#
# Captures, per runtime:
#   - the whole-program bundle (tests/main.tish) across every backend, and
#   - per-test micro-benchmarks (vm / interp / node — the columns the suite prints per file).
#
# Usage:
#   scripts/perf_record.sh [--runtimes R,...] [--runs N] [--timeout SEC] [--out DIR]
#                          [--from-log FILE] [--ref GITREF]
#     --from-log FILE   parse an existing run_performance_suite.sh log instead of running the suite
#                       (used to seed history from a run you already have)
#     --ref GITREF      stamp the record with this ref's commit/date/tag (default: HEAD). Use it when
#                       --from-log came from a different checkout (e.g. a release tag).
#     --out DIR         output directory (default: perf-history)
#   --runtimes/--runs/--timeout are forwarded to run_performance_suite.sh.
#
# Writes: <out>/<commit-date>-<shortsha>.tsv
set -euo pipefail
cd "$(dirname "$0")/.."

runtimes="vm,interp,rust,cranelift,llvm,wasi,node,bun,deno,qjs"
runs=5
timeout=180
out_dir="perf-history"
from_log=""
ref="HEAD"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --runtimes) runtimes="$2"; shift 2 ;;
    --runs) runs="$2"; shift 2 ;;
    --timeout) timeout="$2"; shift 2 ;;
    --out) out_dir="$2"; shift 2 ;;
    --from-log) from_log="$2"; shift 2 ;;
    --ref) ref="$2"; shift 2 ;;
    *) echo "unknown arg: $1" >&2; exit 2 ;;
  esac
done

sha=$(git rev-parse --short "$ref")
# Commit date (not wall-clock) keeps the filename + record deterministic and orderable by history.
cdate=$(git log -1 --format=%cI "$ref")
day=${cdate%%T*}
tag=$(git describe --tags "$ref" 2>/dev/null || echo "")
os=$(uname -sm)

log=""
cleanup=0
if [[ -n "$from_log" ]]; then
  log="$from_log"
  [[ -f "$log" ]] || { echo "ERROR: --from-log '$log' not found" >&2; exit 2; }
else
  log=$(mktemp "${TMPDIR:-/tmp}/tish-perf-rec.XXXXXX")
  cleanup=1
  echo "running perf suite (runtimes=$runtimes runs=$runs)…" >&2
  scripts/run_performance_suite.sh --release --summary-only --timeout "$timeout" \
    --runs "$runs" --runtimes "$runtimes" >"$log" 2>&1 || true
fi

mkdir -p "$out_dir"
record="$out_dir/${day}-${sha}.tsv"

{
  printf '# tish perf record — schema v1 (TSV; ms; lower is better)\n'
  printf '# meta\tcommit\t%s\n' "$sha"
  printf '# meta\tdate\t%s\n' "$cdate"
  printf '# meta\ttag\t%s\n' "$tag"
  printf '# meta\tos\t%s\n' "$os"
  printf '# meta\truntimes\t%s\n' "$runtimes"
  printf '# meta\truns\t%s\n' "$runs"
  printf '# scope\tname\truntime\tms\tstatus\n'

  # Whole-program bundle block: "  Tish (vm):        186ms", "  Node.js:  81ms", … (unique to the
  # final BUNDLED PERF SUITE summary — the "Tish (vm):" form never appears in per-test output).
  awk '
    function ms(s){ if (match(s, /[0-9]+ms/)) { return substr(s, RSTART, RLENGTH-2) } return "" }
    /BUNDLED PERF SUITE/ { inb=1 }
    inb && /Tish \(vm\):/        { print "bundle\ttests/main\tvm\t"        ms($0) "\tok" }
    inb && /Tish \(interp\):/    { print "bundle\ttests/main\tinterp\t"    ms($0) "\tok" }
    inb && /Tish \(rust\):/      { print "bundle\ttests/main\trust\t"      ms($0) "\tok" }
    inb && /Tish \(cranelift\):/ { print "bundle\ttests/main\tcranelift\t" ms($0) "\tok" }
    inb && /Tish \(llvm\):/      { print "bundle\ttests/main\tllvm\t"      ms($0) "\tok" }
    inb && /Tish \(wasi\):/      { print "bundle\ttests/main\twasi\t"      ms($0) "\tok" }
    inb && /Node\.js:/           { print "bundle\ttests/main\tnode\t"      ms($0) "\tok" }
    inb && /^[[:space:]]*Bun:/   { print "bundle\ttests/main\tbun\t"       ms($0) "\tok" }
    inb && /^[[:space:]]*Deno:/  { print "bundle\ttests/main\tdeno\t"      ms($0) "\tok" }
    inb && /QuickJS:/            { print "bundle\ttests/main\tqjs\t"       ms($0) "\tok" }
  ' "$log"

  # Per-test micro-benchmarks: "Running core/x... done (vm: 14ms interp: 13ms node: 37ms …)" and the
  # "FAILED: core/x (vm: …)" variant. The suite only prints vm/interp/node per test.
  awk '
    function pick(line, key,   re){ re = key ": [0-9]+ms"; if (match(line, re)) { s=substr(line,RSTART,RLENGTH); gsub(/[^0-9]/,"",s); return s } return "" }
    function emit(tid, line, st,   v){
      v=pick(line,"vm");     if (v!="") print "test\t" tid "\tvm\t"     v "\t" st
      v=pick(line,"interp"); if (v!="") print "test\t" tid "\tinterp\t" v "\t" st
      v=pick(line,"node");   if (v!="") print "test\t" tid "\tnode\t"   v "\t" st
    }
    /^Running .* done \(vm:/ { tid=$2; sub(/\.\.\.$/,"",tid); emit(tid, $0, "ok") }
    /^FAILED: .* \(vm:/      { tid=$2; emit(tid, $0, "failed") }
  ' "$log"
} > "$record"

[[ $cleanup -eq 1 ]] && rm -f "$log"
rows=$(grep -vc '^#' "$record" || true)
echo "wrote $record ($rows rows; commit $sha${tag:+ / $tag})"
