#!/usr/bin/env bash
# Build the UNTYPED array_records fixture with #350+#351+acctype applied and dump the generated Rust
# (acc decl + read loop) to see why acc stays boxed. Output: target/ar_untyped_rust.txt
set -uo pipefail
cd "$(git rev-parse --show-toplevel)"
OUT=target/ar_untyped_rust.txt; : > "$OUT"
exec > "$OUT" 2>&1

git checkout -- . 2>/dev/null || true
git checkout main 2>/dev/null || true
git branch -D perf/array-records-native-read 2>/dev/null || true

echo "== apply patches as needed =="
grep -q "cse_result_is_boxed" crates/tish_compile/src/codegen.rs || { git apply target/construct.patch && git apply target/cse.patch && echo "applied #350"; }
grep -q "record_array_fields" crates/tish_compile/src/infer.rs || { git apply target/infer.patch && echo "applied #351"; }
git apply target/acctype.patch && echo "applied acctype"

echo "== build tish =="
cargo build --release --bin tish > target/du_build.log 2>&1 || { echo "TISH BUILD FAIL"; tail -25 target/du_build.log; git checkout -- .; exit 0; }

echo "== build UNTYPED array_records into controlled dir =="
rm -rf target/probe/aru; mkdir -p target/probe/aru
TMPDIR="$PWD/target/probe/aru" target/release/tish build tests/perf/array_records.tish -o target/probe/aru_bin > target/probe/aru_build.log 2>&1 || { echo "variant build fail"; tail -20 target/probe/aru_build.log; }
echo "run: $(target/probe/aru_bin 2>&1 || true)"

f=$(find target/probe/aru -name main.rs 2>/dev/null | xargs grep -l "GAUNTLET array_records" 2>/dev/null | head -1 || true)
echo "FILE: $f"
if [ -n "$f" ]; then
  echo "== struct decl =="; grep -nE "struct Tish|TishAnon|Vec<Tish" "$f" | head
  echo "== rows / acc / read loop =="; grep -nE "let mut rows|let mut acc|let acc|acc =|rows\[|\.push\(|ops::|Value::Number\(\(rows" "$f" | head -40
fi
echo "== restore =="
git checkout -- . 2>/dev/null || true
echo DONE
