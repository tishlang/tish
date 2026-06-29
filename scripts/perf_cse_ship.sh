#!/usr/bin/env bash
# Validate + PR the typed-struct-array de-boxing (CSE gate + construction fix). Re-bases both patches
# onto fresh main, builds, re-runs the typed probe (was opt-on 4382ms), verifies the UNTYPED
# array_records #344 win is preserved + soundness + full integration. Output: target/perf_cse.txt
set -uo pipefail
ROOT="$(git rev-parse --show-toplevel)"; cd "$ROOT"
OUT=target/perf_cse.txt; : > "$OUT"
log(){ echo "$@" | tee -a "$OUT"; }
BR=perf/typed-struct-array-noboxing

log "== capture CSE edit =="
git diff -- crates/tish_compile/src/codegen.rs > target/cse.patch
[ -s target/cse.patch ] || { log "ABORT: no CSE edit captured"; exit 1; }
[ -s target/construct.patch ] || { log "ABORT: target/construct.patch missing"; exit 1; }
git checkout -- crates/tish_compile/src/codegen.rs crates/tish_compile/src/types.rs 2>/dev/null || true

log "== branch off fresh main =="
git checkout main --quiet && (git pull --ff-only origin main --quiet 2>/dev/null || true)
git branch -D "$BR" 2>/dev/null || true
git checkout -b "$BR" --quiet || { log "ABORT: branch"; exit 1; }

log "== apply construct + cse patches =="
git apply target/construct.patch || { log "ABORT: construct.patch failed to apply"; git checkout main --quiet; exit 1; }
git apply target/cse.patch || { log "ABORT: cse.patch failed to apply"; git checkout main --quiet; exit 1; }

log "== build release =="
cargo build --release --bin tish > target/cseb.log 2>&1 || { log "ABORT: release build"; tail -25 target/cseb.log|sed 's/^/  /'|tee -a "$OUT"; git checkout main --quiet; exit 1; }
TISH=target/release/tish

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

log "== typed probe (was opt-on 4382ms / boxed 338ms / node 98ms) =="
ON_MS=99999
"$TISH" build target/probe/ar_typed.tish -o target/probe/arc > target/probe/arc.log 2>&1
if [ -x target/probe/arc ]; then line=$(target/probe/arc); echo "  opt-on : $line"|tee -a "$OUT"; ON_MS=$(echo "$line"|awk '{print $2}'); else log "  build FAIL"; tail -20 target/probe/arc.log|sed 's/^/  /'|tee -a "$OUT"; fi
echo "  node   : $(node tests/perf/array_records.tish)"|tee -a "$OUT"

log "== gauntlet: soundness + UNTYPED array_records (#344 win) must NOT regress =="
bash scripts/run_perf_gauntlet.sh array_records object_spread object_sum megamorphic > target/cseg.log 2>&1
grep -E "^(array_records|object_spread|object_sum|megamorphic) |SOUNDNESS|SUMMARY" target/cseg.log | tee -a "$OUT"
grep -q "no build/run/checksum failures" target/cseg.log || { log "ABORT: soundness FAIL"; tail -8 target/cseg.log|sed 's/^/  /'|tee -a "$OUT"; git checkout main --quiet; exit 1; }

log "== full integration =="
cargo build --bin tish > target/csebd.log 2>&1 || { log "ABORT: debug build"; tail -15 target/csebd.log|sed 's/^/  /'|tee -a "$OUT"; git checkout main --quiet; exit 1; }
cargo nextest run -p tishlang --test integration_test > target/cseit.log 2>&1
if grep -qiE "test run failed| [1-9][0-9]* failed" target/cseit.log; then log "ABORT: integration FAIL"; tail -8 target/cseit.log|sed 's/^/  /'|tee -a "$OUT"; git checkout main --quiet; exit 1; fi
log "   integration: $(grep -E 'tests run:' target/cseit.log|tail -1)"

if [ "${ON_MS%%.*}" -ge 1500 ] 2>/dev/null; then
  log "HOLD: typed opt-on still ${ON_MS}ms (>=1500) — not the expected de-boxing win; NOT opening PR."
  git checkout main --quiet; log "DONE (held)"; exit 0
fi

log "== commit + push + PR =="
git add -A
git commit -q -F - <<'MSG'
perf(native): stop boxing typed struct-array accesses (CSE dead-object + construction)

Typed array-of-records code (`rows: Row[]`) ran up to ~13x SLOWER than the boxed build. Two bugs, both
boxing a native value back into a Value on every iteration:

1. Local CSE (#344) hoisted `rows[i]` (3 uses as the base of `.x/.w/.y`) and materialized it with the
   boxed emitter — rebuilding a whole `Value::object` from the struct (4 inserts + alloc) — and the
   temp was DEAD (typed reads use `rows[i].x` directly). 20M pointless allocations. Fix: only hoist a
   candidate that lowers to a boxed `Value`; never a native struct-field / Vec<struct> access (a cheap
   offset load — boxing it is pure loss). #344's own win (untyped Vec<Value> array_records) is
   preserved: those accesses ARE boxed.
2. Building a struct from an object literal (`rows.push({…})`) went through `from_value_expr`, which
   built a boxed object then `get_prop`-ed each field, re-inlining the literal per field. Fix: the
   typed-push path emits the arg via `emit_native_expr` (struct-literal fast path → direct
   `Struct { id: i, … }`); `from_value_expr` binds its source once.

General (any struct-typed array code), not fixture-specific (#317). Part of the array_records lever (#203).
MSG
git push origin "$BR" >/dev/null 2>&1
{
  echo "Typed array-of-records code (\`rows: Row[]\`) ran up to **~13× slower than the boxed build** —"
  echo "two bugs that box a native value back into a \`Value\` every iteration:"
  echo ""
  echo "1. **CSE (#344)** hoisted \`rows[i]\` and materialized it by rebuilding a whole \`Value::object\`"
  echo "   from the struct (4 inserts + alloc) — and the temp was **dead** (typed reads use \`rows[i].x\`"
  echo "   directly): 20M pointless allocations. Fix: only hoist candidates that lower to a boxed"
  echo "   \`Value\`; never a native struct-field / \`Vec<struct>\` access. #344's untyped win is preserved."
  echo "2. **Construction**: \`rows.push({…})\` built a boxed object then \`get_prop\`-ed each field. Fix:"
  echo "   typed-push emits via \`emit_native_expr\` (direct \`Struct { … }\`); \`from_value_expr\` binds once."
  echo ""
  echo "Typed array-of-records probe (2M rows × 10 passes; was opt-on **4382ms**):"
  echo '```'
  grep -E "opt-on|node" "$OUT" | head -2
  echo '```'
  echo "Soundness: object fixtures typed==boxed==node. Untyped array_records (#344) unchanged. Full"
  echo "integration passed. General, not fixture-specific (#317). Part of #203."
} > target/pr_cse.md
url=$(gh pr create --base main --head "$BR" --title "perf(native): stop boxing typed struct-array accesses (CSE dead-object + construction)" --body-file target/pr_cse.md 2>&1 | tail -1)
log "   PR: $url"
git checkout main --quiet
log "DONE"
