#!/usr/bin/env bash
# Locate (or rebuild) the generated Rust for the typed array_records variant and dump it, to see why
# the typed path is 13x slower than boxed. Output: target/ar_rust.txt
set -uo pipefail
cd "$(git rev-parse --show-toplevel)"
OUT=target/ar_rust.txt; : > "$OUT"
exec > "$OUT" 2>&1
f=$(find /var/folders /private/var/folders "${TMPDIR:-/tmp}" /tmp target -path '*build*' -name 'main.rs' 2>/dev/null | xargs grep -l "typed_array_records" 2>/dev/null | head -1 || true)
if [ -z "$f" ]; then
  echo "== not found in caches; rebuild into target/probe/bt =="
  mkdir -p target/probe/bt
  TMPDIR="$PWD/target/probe/bt" target/release/tish build target/probe/ar_typed.tish -o target/probe/ar_typed_bin2 > target/probe/rebuild.log 2>&1 || tail -25 target/probe/rebuild.log
  f=$(find target/probe/bt -name 'main.rs' 2>/dev/null | xargs grep -l "typed_array_records" 2>/dev/null | head -1 || true)
fi
echo "FILE: $f"
[ -n "$f" ] && cat "$f"
echo DONE
