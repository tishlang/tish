#!/usr/bin/env bash
# Validate + PR the array_records FLIP: both part-2 oracle fixes (demote-gate expr_native_type +
# emitter emit_typed_expr handle `arr[i].field`) on top of #350+#351. PRs only if array_records PASSES.
# Output: target/perf_flip.txt
set -uo pipefail
ROOT="$(git rev-parse --show-toplevel)"; cd "$ROOT"
OUT=target/perf_flip.txt; : > "$OUT"
log(){ echo "$@" | tee -a "$OUT"; }
BR=perf/array-records-native-read

log "== capture emit edit =="
git diff -- crates/tish_compile/src/codegen.rs > target/emit.patch
[ -s target/emit.patch ] || { log "ABORT: no emit edit captured"; exit 1; }
[ -s target/acctype.patch ] || { log "ABORT: acctype.patch missing"; exit 1; }
git checkout -- crates/tish_compile/src/codegen.rs 2>/dev/null || true

log "== branch off fresh main =="
git checkout main --quiet && (git pull --ff-only origin main --quiet 2>/dev/null || true)
git branch -D "$BR" 2>/dev/null || true
git checkout -b "$BR" --quiet || { log "ABORT: branch"; exit 1; }

log "== ensure #350 + #351 present =="
grep -q "cse_result_is_boxed" crates/tish_compile/src/codegen.rs || { git apply target/construct.patch && git apply target/cse.patch && log "  applied #350"; }
grep -q "record_array_fields" crates/tish_compile/src/infer.rs || { git apply target/infer.patch && log "  applied #351"; }

log "== apply part-2 patches (demote-gate + typed emit) =="
git apply target/acctype.patch || { log "ABORT: acctype apply"; git checkout main --quiet; exit 1; }
git apply target/emit.patch || { log "ABORT: emit apply"; git checkout main --quiet; exit 1; }

log "== build release =="
cargo build --release --bin tish > target/fb.log 2>&1 || { log "ABORT: release build"; tail -30 target/fb.log|sed 's/^/  /'|tee -a "$OUT"; git checkout main --quiet; exit 1; }

log "== gauntlet: array_records (FLIP?) + neighbors (no regression) =="
bash scripts/run_perf_gauntlet.sh array_records object_sum object_spread megamorphic nbody matmul fasta > target/fg.log 2>&1
grep -E "^(array_records|object_sum|object_spread|megamorphic|nbody|matmul|fasta) |SOUNDNESS|SUMMARY" target/fg.log | tee -a "$OUT"
grep -q "no build/run/checksum failures" target/fg.log || { log "ABORT: soundness FAIL"; tail -10 target/fg.log|sed 's/^/  /'|tee -a "$OUT"; git checkout main --quiet; exit 1; }

log "== full integration =="
cargo build --bin tish > target/fbd.log 2>&1 || { log "ABORT: debug build"; tail -15 target/fbd.log|sed 's/^/  /'|tee -a "$OUT"; git checkout main --quiet; exit 1; }
cargo nextest run -p tishlang --test integration_test > target/fit.log 2>&1
if grep -qiE "test run failed| [1-9][0-9]* failed" target/fit.log; then log "ABORT: integration FAIL"; tail -10 target/fit.log|sed 's/^/  /'|tee -a "$OUT"; git checkout main --quiet; exit 1; fi
log "   integration: $(grep -E 'tests run:' target/fit.log|tail -1)"

AR_ROW=$(grep -E "^array_records " target/fg.log)
log "   array_records: $AR_ROW"
if ! echo "$AR_ROW" | grep -q "PASS"; then
  log "HOLD: array_records still FAIL — not flipped (check the modulo/arithmetic native path). NOT PRing."
  git checkout main --quiet; log "DONE (held)"; exit 0
fi

log "== FLIP — commit + push + PR =="
git add -A
git commit -q -F - <<'MSG'
perf(native): native arithmetic over struct-array field reads — array_records beats node (#203)

Completes the array_records lever. After #351 (untyped→Vec<struct>) + #350 (alloc-free construction),
the read loop `acc = (acc + rows[i].x * rows[i].w + rows[i].y) % …` was still boxed: even with `acc`
native (f64), the struct-array field reads `rows[i].field` emitted boxed, forcing boxed
`ops::add/mul/modulo`. Two oracles only handled an Ident base (`o.field`), not an Index base
(`arr[i].field`):
  * `expr_native_type` (the demote-gate) — so `acc` was demoted to Value;
  * `emit_typed_expr` (the emitter) — so the field reads boxed.
Both now resolve `arr[i].field` to the element struct's native field type → native f64 read loop.
array_records flips to PASS. General (any numeric reduction over an array-of-records), not
fixture-specific (#317).
MSG
git push origin "$BR" >/dev/null 2>&1
{
  echo "Completes the array_records lever. After #351 (untyped→\`Vec<struct>\`) + #350 (alloc-free"
  echo "construction), the read loop was still boxed: even with \`acc\` native, the struct-array field reads"
  echo "\`rows[i].field\` emitted boxed → boxed \`ops::add/mul/modulo\`. Two oracles only handled an Ident base"
  echo "(\`o.field\`), not an Index base (\`arr[i].field\`):"
  echo "- \`expr_native_type\` (demote-gate) → \`acc\` was demoted to Value;"
  echo "- \`emit_typed_expr\` (emitter) → the field reads boxed."
  echo ""
  echo "Both now resolve \`arr[i].field\` to the element struct's native field type → native f64 read loop."
  echo ""
  echo "Gauntlet array_records (2M rows × 10 passes; 937ms boxed → 338ms after #351 → now):"
  echo '```'
  echo "$AR_ROW"
  echo '```'
  echo "Soundness: typed==boxed==node. Full integration passed. Completes the array_records lever (#203)."
} > target/pr_flip.md
url=$(gh pr create --base main --head "$BR" --title "perf(native): native arithmetic over struct-array field reads (array_records beats node)" --body-file target/pr_flip.md 2>&1 | tail -1)
log "   PR: $url"
git checkout main --quiet
log "DONE"
