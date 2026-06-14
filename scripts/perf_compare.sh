#!/usr/bin/env bash
# Diff two tish perf records (perf-history/*.tsv) and report regressions / improvements over time.
#
# The JS engines (node/bun/deno/qjs) run identical .js at both commits, so any move in THEIR numbers
# is pure machine noise. perf_compare derives a noise floor from the bundle's JS-control drift and
# flags a tish backend (vm/interp/rust/cranelift/llvm/wasi) only when it moves MORE than that floor
# (plus a small threshold). Exit status is 1 if any tish backend regressed — usable as a CI gate.
#
# Usage: scripts/perf_compare.sh OLD.tsv NEW.tsv [--threshold PCT] [--min-ms MS]
#   --threshold PCT   minimum |Δ%| to flag, on top of the measured noise floor (default 5)
#   --min-ms MS       ignore a per-test mover unless max(old,new) ≥ MS (default 40). The micro-tests
#                     are ~13ms of fixed process-startup overhead, so a 2-3ms jitter is a meaningless
#                     +20%; only tests heavy enough to be compute-bound carry signal. The bundle
#                     (one ~190ms whole-program run) is never subject to this floor.
set -euo pipefail

[[ $# -ge 2 ]] || { echo "usage: $0 OLD.tsv NEW.tsv [--threshold PCT] [--min-ms MS]" >&2; exit 2; }
old="$1"; new="$2"; shift 2
threshold=5; min_ms=40
while [[ $# -gt 0 ]]; do case "$1" in
  --threshold) threshold="$2"; shift 2 ;;
  --min-ms) min_ms="$2"; shift 2 ;;
  *) echo "unknown: $1" >&2; exit 2 ;;
esac; done
[[ -f "$old" && -f "$new" ]] || { echo "ERROR: record file missing" >&2; exit 2; }

awk -v THR="$threshold" -v MIN="$min_ms" '
  function abs(x){ return x<0?-x:x }
  function iscontrol(rt){ return (rt=="node"||rt=="bun"||rt=="deno"||rt=="qjs") }
  function istish(rt){ return (rt=="vm"||rt=="interp"||rt=="rust"||rt=="cranelift"||rt=="llvm"||rt=="wasi") }
  function swap(a,b,  t){ t=OV[a];OV[a]=OV[b];OV[b]=t; t=NV[a];NV[a]=NV[b];NV[b]=t;
                         t=PCT[a];PCT[a]=PCT[b];PCT[b]=t; t=SC[a];SC[a]=SC[b];SC[b]=t;
                         t=RT[a];RT[a]=RT[b];RT[b]=t; t=NAME[a];NAME[a]=NAME[b];NAME[b]=t }
  BEGIN{ FS="\t" }
  $1=="# meta"{ if(FNR==NR) om[$2]=$3; else nm[$2]=$3; next }
  /^#/{ next }
  FNR==NR{ old[$1"|"$2"|"$3]=$4; next }
  {
    key=$1"|"$2"|"$3; if(!(key in old)) next
    o=old[key]+0; n=$4+0; if(o<=0) next
    pct=100.0*(n-o)/o
    if($1=="bundle" && iscontrol($3) && abs(pct)>floor) floor=abs(pct)
    m++; OV[m]=o; NV[m]=n; PCT[m]=pct; SC[m]=$1; RT[m]=$3; NAME[m]=$2
  }
  END{
    eff = (THR>floor)?THR:floor
    printf "perf-over-time diff\n"
    printf "  old: %-9s %-22s %s\n", om["commit"], om["tag"], om["date"]
    printf "  new: %-9s %-22s %s\n", nm["commit"], nm["tag"], nm["date"]
    printf "  noise floor (bundle JS-control drift) = %.1f%%  → flag tish moves > %.1f%%\n\n", floor, eff

    printf "BUNDLE (tests/main, ms)\n"
    printf "  %-10s %8s %8s %9s\n", "runtime","old","new","delta"
    split("vm interp rust cranelift llvm wasi node bun deno qjs", ob, " ")
    for(i=1;i<=10;i++) for(j=1;j<=m;j++) if(SC[j]=="bundle" && RT[j]==ob[i])
      printf "  %-10s %8.0f %8.0f %+8.1f%%%s\n", RT[j], OV[j], NV[j], PCT[j], istish(RT[j])?"":"  (control)"
    printf "\n"

    for(a=1;a<=m;a++) for(b=a+1;b<=m;b++) if(PCT[b]>PCT[a]) swap(a,b)
    nreg=0; nimp=0; any=0
    printf "TISH MOVERS (|delta| > %.1f%%, tish backends, max(old,new) >= %dms)\n", eff, MIN
    for(j=1;j<=m;j++){
      if(!istish(RT[j]) || abs(PCT[j])<=eff) continue
      if(SC[j]!="bundle" && (OV[j]<MIN && NV[j]<MIN)) continue   # startup-bound micro-test → skip
      any=1; if(PCT[j]>0) nreg++; else nimp++
      printf "  %-9s %-28s %6.0f -> %-6.0f %+7.1f%%  %s\n", RT[j],
        (SC[j]=="bundle"?"tests/main (bundle)":NAME[j]), OV[j], NV[j], PCT[j], (PCT[j]>0?"SLOWER":"faster")
    }
    if(!any) printf "  (none — every tish backend is within the noise floor)\n"
    printf "\nSUMMARY: %d slower, %d faster beyond a %.1f%% floor, over %d compared points\n", nreg, nimp, eff, m
    exit (nreg>0)?1:0
  }
' "$old" "$new"
