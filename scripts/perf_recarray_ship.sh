#!/usr/bin/env bash
# Validate + PR the array-of-records inference. Re-bases the infer.rs edit onto fresh main (applying
# #350's de-boxing patches if not yet merged), builds, checks whether untyped array_records lowers to
# Vec<struct> and drops from ~937ms, with soundness + full integration. PRs only on a clear win.
# Output: target/perf_recarray.txt
set -uo pipefail
ROOT="$(git rev-parse --show-toplevel)"; cd "$ROOT"
OUT=target/perf_recarray.txt; : > "$OUT"
log(){ echo "$@" | tee -a "$OUT"; }
BR=perf/array-of-records-infer

log "== capture infer.rs edit =="
git diff -- crates/tish_compile/src/infer.rs > target/infer.patch
[ -s target/infer.patch ] || { log "ABORT: no infer edit captured"; exit 1; }
git checkout -- crates/tish_compile/src/infer.rs 2>/dev/null || true

log "== branch off fresh main =="
git checkout main --quiet && (git pull --ff-only origin main --quiet 2>/dev/null || true)
git branch -D "$BR" 2>/dev/null || true
git checkout -b "$BR" --quiet || { log "ABORT: branch"; exit 1; }

log "== ensure #350 de-boxing present =="
if grep -q "cse_result_is_boxed" crates/tish_compile/src/codegen.rs; then
  log "  #350 already in main"
else
  log "  #350 not merged yet — applying construct + cse patches"
  git apply target/construct.patch && git apply target/cse.patch || { log "ABORT: #350 patch failed"; git checkout main --quiet; exit 1; }
fi

log "== apply infer patch =="
git apply target/infer.patch || { log "ABORT: infer patch failed to apply"; git checkout main --quiet; exit 1; }

log "== build release =="
cargo build --release --bin tish > target/rab.log 2>&1 || { log "ABORT: release build"; tail -30 target/rab.log|sed 's/^/  /'|tee -a "$OUT"; git checkout main --quiet; exit 1; }

log "== gauntlet: array_records (target) + object/struct fixtures (no regression) =="
bash scripts/run_perf_gauntlet.sh array_records object_sum object_spread megamorphic fasta > target/rag.log 2>&1
grep -E "^(array_records|object_sum|object_spread|megamorphic|fasta) |SOUNDNESS|SUMMARY" target/rag.log | tee -a "$OUT"
grep -q "no build/run/checksum failures" target/rag.log || { log "ABORT: soundness FAIL"; tail -10 target/rag.log|sed 's/^/  /'|tee -a "$OUT"; git checkout main --quiet; exit 1; }

log "== full integration =="
cargo build --bin tish > target/rabd.log 2>&1 || { log "ABORT: debug build"; tail -15 target/rabd.log|sed 's/^/  /'|tee -a "$OUT"; git checkout main --quiet; exit 1; }
cargo nextest run -p tishlang --test integration_test > target/rait.log 2>&1
if grep -qiE "test run failed| [1-9][0-9]* failed" target/rait.log; then log "ABORT: integration FAIL"; tail -10 target/rait.log|sed 's/^/  /'|tee -a "$OUT"; git checkout main --quiet; exit 1; fi
log "   integration: $(grep -E 'tests run:' target/rait.log|tail -1)"

AR_TYPED=$(grep -E "^array_records " target/rag.log | awk '{print $3}' | tr -d 'ms')
log "   array_records typed-on = ${AR_TYPED:-?}ms (was ~937ms)"
if [ -z "$AR_TYPED" ] || ! [ "${AR_TYPED%%.*}" -lt 700 ] 2>/dev/null; then
  log "HOLD: array_records not clearly improved (>=700ms or unparsed) — inference didn't fire/help; NOT PRing."
  git checkout main --quiet; log "DONE (held)"; exit 0
fi

log "== commit + push + PR =="
git add -A
git commit -q -F - <<'MSG'
perf(native): infer untyped arrays-of-records as native Vec<struct> (#203)

`let rows = []; for (…) rows.push({ id, x, y, w }); … rows[i].x …` — the canonical "array of records"
shape — stayed a boxed `Value[]` of boxed objects (per-element hashing + cloning). The struct-inference
pass (`si_block`) already lowered single object locals and native scalar arrays; this adds the
array-of-records case: when a local array is built only by `push({uniform primitive object literal})`
and used only as `name[i].<field>` reads / `name.length`, infer it as `Vec<struct>` (shape synthesized
+ interned like the single-object path). The struct codegen then emits offset field access and
(post-#350) allocation-free construction. Conservative + sound — any other use of the binding leaves
it boxed (mirrors the #177 S-D safety walk); typed==boxed==node preserved. General, not
fixture-specific (#317).
MSG
git push origin "$BR" >/dev/null 2>&1
{
  echo "\`let rows = []; for (…) rows.push({ id, x, y, w }); … rows[i].x …\` — the canonical array-of-records"
  echo "shape — stayed a boxed \`Value[]\` of boxed objects (per-element hashing + cloning)."
  echo ""
  echo "The struct-inference pass already lowered single object locals + native scalar arrays; this adds the"
  echo "**array-of-records** case: a local array built only by \`push({uniform primitive object literal})\` and"
  echo "used only as \`name[i].<field>\` reads / \`name.length\` is inferred as \`Vec<struct>\` (shape synthesized +"
  echo "interned). The struct codegen emits offset field access + (post-#350) allocation-free construction."
  echo "Conservative + sound (mirrors the #177 S-D walk); any other use of the binding stays boxed."
  echo ""
  echo "Gauntlet array_records (2M rows × 10 passes; was ~937ms typed-on, 9.8× node):"
  echo '```'
  grep -E "^array_records " target/rag.log
  echo '```'
  echo "Soundness: typed==boxed==node. Full integration passed. General, not fixture-specific (#317). #203."
} > target/pr_recarray.md
url=$(gh pr create --base main --head "$BR" --title "perf(native): infer untyped arrays-of-records as native Vec<struct>" --body-file target/pr_recarray.md 2>&1 | tail -1)
log "   PR: $url"
git checkout main --quiet
log "DONE"
