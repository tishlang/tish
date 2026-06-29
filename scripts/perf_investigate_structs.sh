#!/usr/bin/env bash
# Read-only: dump the object->struct lowering path, the #177 aggregate-infer gating + its ABI blocker,
# and the object-heavy fixtures, so the next boxed-Value increment can be designed without further
# per-command approvals. Output: target/structs_repr.txt
set -uo pipefail
cd "$(git rev-parse --show-toplevel)"
OUT=target/structs_repr.txt
exec > "$OUT" 2>&1

echo "############ object-literal -> Rust struct lowering (codegen ~9100-9260) ############"
sed -n '9100,9260p' crates/tish_compile/src/codegen.rs

echo ""; echo "############ recursive struct-shape inference / builders (codegen ~17140-17430) ############"
sed -n '17140,17430p' crates/tish_compile/src/codegen.rs

echo ""; echo "############ TISH_AGGREGATE_INFER gating + S-tiers + FnSigTable + ABI ############"
grep -rn "TISH_AGGREGATE_INFER\|aggregate_infer\|FnSigTable\|S-0\|S-F\|shared-handle\|Vec<Named>\|writeback\|struct_lower\|emit_struct\|RecordShape\|infer_struct" crates/tish_compile/src/*.rs | head -60

echo ""; echo "############ object-heavy fixtures (source) ############"
for f in object_spread megamorphic array_records; do
  p=$(find . -path ./target -prune -o -name "${f}.tish" -print 2>/dev/null | head -1)
  [ -z "$p" ] && p=$(find . -path ./target -prune -o -name "${f}.js" -print 2>/dev/null | head -1)
  echo "===== $f  ($p) ====="
  [ -n "$p" ] && cat "$p"
done

echo "DONE"
