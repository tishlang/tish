#!/usr/bin/env bash
set -uo pipefail
cd "$(git rev-parse --show-toplevel)"
OUT=target/sb_rust.txt; : > "$OUT"; exec > "$OUT" 2>&1
rm -rf target/probe/sb; mkdir -p target/probe/sb
TMPDIR="$PWD/target/probe/sb" target/release/tish build tests/perf/string_build.tish -o target/probe/sb_bin > target/probe/sb_build.log 2>&1 || { echo "build fail"; tail -20 target/probe/sb_build.log; }
echo "run: $(target/probe/sb_bin 2>&1 || true)"
f=$(find target/probe/sb -name main.rs 2>/dev/null | xargs grep -l "GAUNTLET string_build" 2>/dev/null | head -1 || true)
echo "FILE: $f"
[ -n "$f" ] && { echo "== acc / parts / += / join / push =="; grep -nE "let mut acc|let mut parts|acc =|\.push_str|push\(|join|ops::add|ops::concat|Value::String|string_concat|to_display_string|\+= " "$f" | head -40; }
echo DONE
