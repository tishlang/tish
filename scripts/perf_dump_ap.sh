#!/usr/bin/env bash
set -uo pipefail
cd "$(git rev-parse --show-toplevel)"
OUT=target/ap_rust.txt; : > "$OUT"; exec > "$OUT" 2>&1
rm -rf target/probe/ap; mkdir -p target/probe/ap
TMPDIR="$PWD/target/probe/ap" target/release/tish build tests/perf/array_pipeline.tish -o target/probe/ap_bin > target/probe/ap_build.log 2>&1 || { echo "build fail"; tail -20 target/probe/ap_build.log; }
echo "run: $(target/probe/ap_bin 2>&1 || true)"
f=$(find target/probe/ap -name main.rs 2>/dev/null | xargs grep -l "GAUNTLET array_pipeline" 2>/dev/null | head -1 || true)
echo "FILE: $f"
[ -n "$f" ] && { echo "== data decl + fused pipeline loop =="; grep -nE "let mut data|NumberArray|Vec<f64>|let mut check|let mut total|filter|\.map|reduce|for_loop|borrow|ops::|value_call|% 3|x \* 2|2147483647|\.iter\(\)" "$f" | head -45; }
echo DONE
