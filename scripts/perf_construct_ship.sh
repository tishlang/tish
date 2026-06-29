#!/usr/bin/env bash
# Validate + PR the direct-struct-construction fix. Re-bases the codegen/types edits onto fresh main,
# builds, re-runs the typed array-of-records probe, checks soundness + full integration, PRs only on a
# real win. One run, no approvals. Output: target/perf_construct.txt
set -uo pipefail
ROOT="$(git rev-parse --show-toplevel)"; cd "$ROOT"
OUT=target/perf_construct.txt; : > "$OUT"
log(){ echo "$@" | tee -a "$OUT"; }
BR=perf/struct-construct-direct

log "== capture edits =="
git diff -- crates/tish_compile/src/codegen.rs crates/tish_compile/src/types.rs > target/construct.patch
[ -s target/construct.patch ] || { log "ABORT: no edit captured"; exit 1; }
git checkout -- crates/tish_compile/src/codegen.rs crates/tish_compile/src/types.rs

log "== branch off fresh main =="
git checkout main --quiet && (git pull --ff-only origin main --quiet 2>/dev/null || true)
git branch -D "$BR" 2>/dev/null || true
git checkout -b "$BR" --quiet || { log "ABORT: branch"; exit 1; }
git apply target/construct.patch || { log "ABORT: patch apply on main"; git checkout main --quiet; exit 1; }

log "== build release =="
cargo build --release --bin tish > target/cb.log 2>&1 || { log "ABORT: release build"; tail -25 target/cb.log|sed 's/^/  /'|tee -a "$OUT"; git checkout main --quiet; exit 1; }
TISH=target/release/tish

# ensure the typed probe source exists
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

log "== typed array-of-records probe (was: opt-on 4401ms / boxed 340ms / node 100ms) =="
ON_MS=99999
"$TISH" build target/probe/ar_typed.tish -o target/probe/ar2 > target/probe/b1.log 2>&1
if [ -x target/probe/ar2 ]; then
  line=$(target/probe/ar2); echo "  opt-on : $line" | tee -a "$OUT"; ON_MS=$(echo "$line" | awk '{print $2}')
else log "  typed opt-on build FAIL"; tail -20 target/probe/b1.log | sed 's/^/  /' | tee -a "$OUT"; fi
TISH_NATIVE_OPT=0 "$TISH" build target/probe/ar_typed.tish -o target/probe/ar2b > target/probe/b2.log 2>&1 && echo "  boxed  : $(target/probe/ar2b)" | tee -a "$OUT" || true
echo "  node   : $(node tests/perf/array_records.tish)" | tee -a "$OUT"

log "== gauntlet soundness (object fixtures — no regression) =="
bash scripts/run_perf_gauntlet.sh object_spread array_records object_sum megamorphic > target/cg.log 2>&1
grep -E "^(object_spread|array_records|object_sum|megamorphic) |SOUNDNESS|SUMMARY" target/cg.log | tee -a "$OUT"
grep -q "no build/run/checksum failures" target/cg.log || { log "ABORT: soundness FAIL"; tail -8 target/cg.log|sed 's/^/  /'|tee -a "$OUT"; git checkout main --quiet; exit 1; }

log "== full integration =="
cargo build --bin tish > target/cbd.log 2>&1 || { log "ABORT: debug build"; tail -15 target/cbd.log|sed 's/^/  /'|tee -a "$OUT"; git checkout main --quiet; exit 1; }
cargo nextest run -p tishlang --test integration_test > target/cit.log 2>&1
if grep -qiE "test run failed| [1-9][0-9]* failed" target/cit.log; then log "ABORT: integration FAIL"; tail -8 target/cit.log|sed 's/^/  /'|tee -a "$OUT"; git checkout main --quiet; exit 1; fi
log "   integration: $(grep -E 'tests run:' target/cit.log|tail -1)"

# gate: only PR if the typed probe is now a clear win (was 4401ms)
if [ "${ON_MS%%.*}" -ge 1500 ] 2>/dev/null; then
  log "HOLD: typed opt-on still ${ON_MS}ms (>=1500) — not a clear win yet; NOT opening PR. Investigate read-loop/acc boxing next."
  git checkout main --quiet; log "== DONE (held) =="; exit 0
fi

log "== commit + push + PR =="
git add -A
git commit -q -F - <<'MSG'
perf(native): construct typed structs directly from object literals (no boxed round-trip)

Building a typed record from an object literal — `rows.push({ id, x, y, w })` where `rows: Row[]` —
lowered to a full boxed `Value::object_from_pairs(...)` that was then `get_prop`-ed field by field,
and `from_value_expr` re-inlined that whole object once PER FIELD. A 4-field record cost four object
allocations + four hash lookups per element. Fixes: (1) the typed-`push` fast path emits the argument
via `emit_native_expr(elem_type)`, so an object literal hits the existing struct-literal fast path
(direct `Struct { id: i, x: i*2, … }`, zero allocations); (2) `from_value_expr` for a `Named` struct
binds the source Value once (`let _src = …`) instead of re-inlining per field. General (any struct-typed
code), not fixture-specific (#317). First step of the array_records lever (#203); the read path was
already offset-based.
MSG
git push origin "$BR" >/dev/null 2>&1
{
  echo "Building a typed record from an object literal — \`rows.push({ id, x, y, w })\` with \`rows: Row[]\` —"
  echo "lowered to a boxed \`Value::object_from_pairs(...)\` that was then \`get_prop\`-ed field by field, and"
  echo "\`from_value_expr\` re-inlined that whole object **once per field**: a 4-field record cost **4 object"
  echo "allocations + 4 hash lookups per element**."
  echo ""
  echo "Fixes: (1) the typed-\`push\` fast path emits the arg via \`emit_native_expr(elem_type)\` so an object"
  echo "literal hits the struct-literal fast path (direct \`Struct { id: i, x: i*2, … }\`, zero allocs);"
  echo "(2) \`from_value_expr\` for a \`Named\` struct binds the source once (\`let _src = …\`) instead of"
  echo "re-inlining per field. General, not fixture-specific (#317)."
  echo ""
  echo "Typed array-of-records probe (2M rows × 10 passes; was opt-on 4401ms):"
  echo '```'
  grep -E "opt-on|boxed|node" "$OUT" | head -3
  echo '```'
  echo "Soundness: object fixtures typed==boxed==node. Full integration passed."
} > target/pr_construct.md
url=$(gh pr create --base main --head "$BR" --title "perf(native): construct typed structs directly from object literals (no boxed round-trip)" --body-file target/pr_construct.md 2>&1 | tail -1)
log "   PR: $url"
git checkout main --quiet
log "== DONE =="
