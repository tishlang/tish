#!/usr/bin/env bash
# Render a perf-history TSV into a human-readable Markdown table — ALL runtimes, ALL tests, raw ms,
# no comparison or interpretation. So a release record can be read directly instead of via a tool.
#
# Usage: scripts/perf_render_md.sh perf-history/<date>-<sha>.tsv > perf-history/<date>-<sha>.md
set -euo pipefail
[[ $# -ge 1 && -f "$1" ]] || { echo "usage: $0 RECORD.tsv" >&2; exit 2; }

awk -F'\t' '
  $1=="# meta" { meta[$2]=$3; next }
  /^#/ { next }
  $1=="bundle" { brt[$3]=$4; bst[$3]=$5; if(!seen[$3]++) order[++nrt]=$3; next }
  {  # per-file micro rows: scope=test
     key=$1"/"$2; if(!(key in trow)) tlist[++nt]=key
     mic[key,$3]=$4
     if(!seen[$3]++) order[++nrt]=$3
     trow[key]=1
  }
  END {
    printf "# Perf record — %s (%s)\n\n", (meta["tag"]!=""?meta["tag"]:meta["commit"]), meta["commit"]
    printf "- date: `%s`\n- os: `%s`\n- runs: `%s`\n- runtimes recorded: `%s`\n\n", \
           meta["date"], meta["os"], meta["runs"], meta["runtimes"]
    print "_Raw timings in milliseconds. No comparison or normalization — this is the logged record._\n"

    # Fixed display order; only emit runtimes actually present.
    n=split("vm interp rust cranelift llvm wasi node bun deno qjs", pref, " ")

    print "## Whole-program bundle (`tests/main`)\n"
    print "| runtime | ms | status |"
    print "|---|---:|---|"
    for(i=1;i<=n;i++){ r=pref[i]; if(r in brt) printf "| %s | %s | %s |\n", r, brt[r], bst[r] }
    print ""

    print "## Per-file micro-benchmarks\n"
    # Header: test + each present runtime that has at least one micro value.
    hdr="| test"; sep="|---"
    delete present
    for(i=1;i<=n;i++){ r=pref[i]; for(t=1;t<=nt;t++){ if((tlist[t] SUBSEP r) in mic){ present[r]=1; break } } }
    for(i=1;i<=n;i++){ r=pref[i]; if(r in present){ hdr=hdr" | "r; sep=sep"|---:" } }
    print hdr" |"; print sep"|"
    # Sort tests alphabetically for stable output.
    for(t=1;t<=nt;t++) idx[t]=tlist[t]
    for(a=1;a<=nt;a++) for(b=a+1;b<=nt;b++) if(idx[a]>idx[b]){ tmp=idx[a]; idx[a]=idx[b]; idx[b]=tmp }
    for(t=1;t<=nt;t++){
      key=idx[t]; line="| "key
      for(i=1;i<=n;i++){ r=pref[i]; if(r in present){ v=((key SUBSEP r) in mic)?mic[key,r]:"·"; line=line" | "v } }
      print line" |"
    }
  }
' "$1"
