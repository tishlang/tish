#!/usr/bin/env bash
# Clean git state, rebuild main's tish, build the opt-on typed array_records variant into a controlled
# dir, and dump its generated main.rs to find the 13x pessimization. Output: target/ar_opton_rust.txt
set -uo pipefail
cd "$(git rev-parse --show-toplevel)"
OUT=target/ar_opton_rust.txt; : > "$OUT"
exec > "$OUT" 2>&1

echo "== git cleanup (construct edits are saved in target/construct.patch) =="
git checkout -- . 2>/dev/null || true
git checkout main 2>/dev/null || true
git branch -D perf/struct-construct-direct 2>/dev/null || true
echo "on branch: $(git branch --show-current)"

echo "== rebuild release tish (main) =="
cargo build --release --bin tish > target/rb.log 2>&1 || { echo "TISH BUILD FAIL"; tail -20 target/rb.log; exit 0; }

mkdir -p target/probe
cat > target/probe/ar_typed.tish <<'TISH'
type Row = { id: number, x: number, y: number, w: number }
let n = 2000000
let rows: Row[] = []
for (let i = 0; i < n; i++) { rows.push({ id: i, x: i * 2, y: i + 1, w: (i % 7) + 1 }) }
let t0 = Date.now()
let acc = 0
for (let pass = 0; pass < 10; pass++) {
  for (let i = 0; i < rows.length; i++) {
    acc = (acc + rows[i].x * rows[i].w + rows[i].y) % 2147483647
  }
}
let CHECK = acc % 2147483647
console.log("typed_array_records " + (Date.now() - t0) + " " + CHECK)
TISH

echo "== build opt-on typed variant into controlled TMPDIR =="
rm -rf target/probe/opton; mkdir -p target/probe/opton
TMPDIR="$PWD/target/probe/opton" target/release/tish build target/probe/ar_typed.tish -o target/probe/aropton > target/probe/opton_build.log 2>&1 || { echo "variant build fail"; tail -20 target/probe/opton_build.log; }
f=$(find target/probe/opton "${TMPDIR:-/tmp}" /var/folders /private/var/folders -path '*build*' -name 'main.rs' 2>/dev/null | xargs grep -l typed_array_records 2>/dev/null | head -1 || true)
echo "FILE: $f"
[ -n "$f" ] && cat "$f"
echo DONE
