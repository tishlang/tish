#!/usr/bin/env bash
# Re-base the working-tree codegen edit onto fresh main, build, validate (gauntlet soundness + full
# integration), and PR — all in one run, no per-command approvals. Output: target/perf_spread.txt
set -uo pipefail
ROOT="$(git rev-parse --show-toplevel)"; cd "$ROOT"
OUT=target/perf_spread.txt; : > "$OUT"
log(){ echo "$@" | tee -a "$OUT"; }
BR=perf/object-spread-shape-clone

log "== capture working edit as a patch =="
git diff -- crates/tish_compile/src/codegen.rs > target/spread.patch
if [ ! -s target/spread.patch ]; then log "ABORT: no edit captured (working tree clean?)"; exit 1; fi
git checkout -- crates/tish_compile/src/codegen.rs

log "== branch off fresh main =="
git checkout main --quiet && (git pull --ff-only origin main --quiet 2>/dev/null || true)
git branch -D "$BR" 2>/dev/null || true
git checkout -b "$BR" --quiet || { log "ABORT: branch create failed"; exit 1; }

log "== apply edit onto main =="
if ! git apply target/spread.patch; then log "ABORT: patch did not apply on main (spread region diverged)"; git checkout main --quiet; exit 1; fi

log "== build release =="
if ! cargo build --release --bin tish > target/build_spread.log 2>&1; then
  log "ABORT: release build failed"; tail -25 target/build_spread.log | sed 's/^/  /' | tee -a "$OUT"; git checkout main --quiet; exit 1
fi

log "== gauntlet (object_spread + object-heavy neighbors) =="
bash scripts/run_perf_gauntlet.sh object_spread megamorphic array_records object_sum > target/gt_spread.log 2>&1
grep -E "^(object_spread|megamorphic|array_records|object_sum) |SOUNDNESS|SUMMARY" target/gt_spread.log | tee -a "$OUT"
if ! grep -q "no build/run/checksum failures" target/gt_spread.log; then
  log "ABORT: soundness FAILED — not shipping"; tail -8 target/gt_spread.log | sed 's/^/  /' | tee -a "$OUT"; git checkout main --quiet; exit 1
fi
log "   soundness OK"

log "== full integration =="
if ! cargo build --bin tish > target/build_spread_dbg.log 2>&1; then
  log "ABORT: debug build failed"; tail -15 target/build_spread_dbg.log | sed 's/^/  /' | tee -a "$OUT"; git checkout main --quiet; exit 1
fi
cargo nextest run -p tishlang --test integration_test > target/it_spread.log 2>&1
if grep -qiE "test run failed| [1-9][0-9]* failed" target/it_spread.log; then
  log "ABORT: integration FAILED"; tail -8 target/it_spread.log | sed 's/^/  /' | tee -a "$OUT"; git checkout main --quiet; exit 1
fi
log "   integration: $(grep -E 'tests run:' target/it_spread.log | tail -1)"

log "== commit + push + PR =="
git add -A
git commit -q -F - <<'MSG'
perf(native): shape-preserving object spread — clone base PropMap instead of per-key merge

The `{ ...base, k: v }` immutable-update pattern (Redux/React/config-merge) rebuilt the
result PropMap by re-inserting every base key via `merge_from` — each insert pays a dedup
scan plus a hidden-class shape transition — on every evaluation. When the first property
is a spread, seed the result by cloning the source PropMap wholesale: a structural copy
that carries the shape id across and skips the per-key work. Override semantics are
unchanged (later props apply on top in source order); a non-object spread seeds an empty
map, matching JS. General, not fixture-specific (#317).
MSG
git push origin "$BR" >/dev/null 2>&1
{
  echo "The \`{ ...base, k: v }\` immutable-update pattern (Redux/React/config-merge) rebuilt the result"
  echo "\`PropMap\` by re-inserting **every** base key via \`merge_from\` — each insert pays a dedup scan"
  echo "plus a hidden-class \`shape\` transition — on every evaluation."
  echo ""
  echo "When the first property is a spread, seed the result by **cloning the source \`PropMap\`"
  echo "wholesale**: a structural copy that carries the hidden-class \`shape\` id across and skips the"
  echo "per-key re-insert. Later properties apply on top in source order, so override semantics are"
  echo "unchanged; a non-object spread (\`{...null}\`) seeds an empty map, matching JS. General, not"
  echo "fixture-specific (#317)."
  echo ""
  echo "Gauntlet (this run):"
  echo '```'
  grep -E "^(object_spread|megamorphic|array_records|object_sum) " target/gt_spread.log
  echo '```'
  echo "Soundness: typed==boxed==node. Full integration passed."
} > target/pr_spread.md
url=$(gh pr create --base main --head "$BR" --title "perf(native): shape-preserving object spread (clone base PropMap, skip per-key merge)" --body-file target/pr_spread.md 2>&1 | tail -1)
log "   PR: $url"
git checkout main --quiet
log "== DONE =="
