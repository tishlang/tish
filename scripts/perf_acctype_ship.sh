#!/usr/bin/env bash
# Validate + PR the demote-gate fix (native numeric accumulators across struct-array field reads).
# Re-bases onto fresh main (applying #351 inference + #350 de-boxing if not yet merged), builds, and
# checks whether array_records now FLIPS (beats node → PASS) with soundness + full integration.
# Output: target/perf_acctype.txt
set -uo pipefail
ROOT="$(git rev-parse --show-toplevel)"; cd "$ROOT"
OUT=target/perf_acctype.txt; : > "$OUT"
log(){ echo "$@" | tee -a "$OUT"; }
BR=perf/array-records-native-read

log "== capture codegen edit =="
git diff -- crates/tish_compile/src/codegen.rs > target/acctype.patch
[ -s target/acctype.patch ] || { log "ABORT: no edit captured"; exit 1; }
git checkout -- crates/tish_compile/src/codegen.rs 2>/dev/null || true

log "== branch off fresh main =="
git checkout main --quiet && (git pull --ff-only origin main --quiet 2>/dev/null || true)
git branch -D "$BR" 2>/dev/null || true
git checkout -b "$BR" --quiet || { log "ABORT: branch"; exit 1; }

log "== ensure #350 + #351 present =="
if ! grep -q "cse_result_is_boxed" crates/tish_compile/src/codegen.rs; then
  log "  applying #350 (construct + cse)"; git apply target/construct.patch && git apply target/cse.patch || { log "ABORT #350"; git checkout main --quiet; exit 1; }
fi
if grep -q "record_array_fields" crates/tish_compile/src/infer.rs; then
  log "  #351 already in main"
else
  log "  applying #351 infer patch"; git apply target/infer.patch || { log "ABORT: #351 infer patch failed"; git checkout main --quiet; exit 1; }
fi

log "== apply demote-gate patch =="
git apply target/acctype.patch || { log "ABORT: acctype patch failed"; git checkout main --quiet; exit 1; }

log "== build release =="
cargo build --release --bin tish > target/atb.log 2>&1 || { log "ABORT: release build"; tail -30 target/atb.log|sed 's/^/  /'|tee -a "$OUT"; git checkout main --quiet; exit 1; }

log "== gauntlet: array_records (FLIP?) + neighbors (no regression) =="
bash scripts/run_perf_gauntlet.sh array_records object_sum object_spread megamorphic nbody matmul > target/atg.log 2>&1
grep -E "^(array_records|object_sum|object_spread|megamorphic|nbody|matmul) |SOUNDNESS|SUMMARY" target/atg.log | tee -a "$OUT"
grep -q "no build/run/checksum failures" target/atg.log || { log "ABORT: soundness FAIL"; tail -10 target/atg.log|sed 's/^/  /'|tee -a "$OUT"; git checkout main --quiet; exit 1; }

log "== full integration =="
cargo build --bin tish > target/atbd.log 2>&1 || { log "ABORT: debug build"; tail -15 target/atbd.log|sed 's/^/  /'|tee -a "$OUT"; git checkout main --quiet; exit 1; }
cargo nextest run -p tishlang --test integration_test > target/atit.log 2>&1
if grep -qiE "test run failed| [1-9][0-9]* failed" target/atit.log; then log "ABORT: integration FAIL"; tail -10 target/atit.log|sed 's/^/  /'|tee -a "$OUT"; git checkout main --quiet; exit 1; fi
log "   integration: $(grep -E 'tests run:' target/atit.log|tail -1)"

AR_ROW=$(grep -E "^array_records " target/atg.log)
log "   array_records: $AR_ROW"
if ! echo "$AR_ROW" | grep -q "PASS"; then
  log "HOLD: array_records still FAIL — native read didn't flip it. NOT opening PR (improvement-only)."
  git checkout main --quiet; log "DONE (held)"; exit 0
fi

log "== FLIP confirmed — commit + push + PR =="
git add -A
git commit -q -F - <<'MSG'
perf(native): native numeric accumulators across struct-array field reads (array_records beats node)

After #351 lowered untyped arrays-of-records to `Vec<struct>`, the array_records read loop was still
boxed: the `number` accumulator `acc` got demoted to `Value` because the demote-gate oracle
(`expr_native_type`) resolved `var.field` (Ident base) but NOT `arr[i].field` (Index base into a
`Vec<struct>`). So `acc = (acc + rows[i].x * rows[i].w + rows[i].y) % …` typed as Value and the whole
loop boxed. Fix: teach `expr_native_type`'s Member arm the `arr[i].field` case — the element struct's
native field type — so the accumulator stays `f64` and the read loop emits native arithmetic. General
(any numeric reduction over an array-of-records), not fixture-specific (#317). Completes the
array_records lever (#203).
MSG
git push origin "$BR" >/dev/null 2>&1
{
  echo "After #351 lowered untyped arrays-of-records to \`Vec<struct>\`, the read loop was still boxed: the"
  echo "\`number\` accumulator \`acc\` got demoted to \`Value\` because the demote-gate oracle (\`expr_native_type\`)"
  echo "resolved \`var.field\` (Ident base) but NOT \`arr[i].field\` (Index base into a \`Vec<struct>\`)."
  echo ""
  echo "Fix: teach \`expr_native_type\`'s Member arm the \`arr[i].field\` case (the element struct's native"
  echo "field type), so the accumulator stays \`f64\` and the read loop emits native arithmetic. General"
  echo "(any numeric reduction over an array-of-records), not fixture-specific (#317)."
  echo ""
  echo "Gauntlet array_records (2M rows × 10 passes; was 937ms boxed → 338ms after #351 → now):"
  echo '```'
  echo "$AR_ROW"
  echo '```'
  echo "Soundness: typed==boxed==node. Full integration passed. Completes the array_records lever (#203)."
} > target/pr_acctype.md
url=$(gh pr create --base main --head "$BR" --title "perf(native): native numeric accumulators across struct-array field reads (array_records beats node)" --body-file target/pr_acctype.md 2>&1 | tail -1)
log "   PR: $url"
git checkout main --quiet
log "DONE"
