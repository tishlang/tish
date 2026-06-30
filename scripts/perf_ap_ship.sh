#!/usr/bin/env bash
# Validate + PR the array_pipeline fix (native Vec<f64> HOF source → no per-pass snapshot). Touches
# number-array HOF inference, so sweep the HOF/number-array fixtures for regressions; PR only if
# array_pipeline flips (PASS) with soundness + full integration. Output: target/perf_ap.txt
set -uo pipefail
ROOT="$(git rev-parse --show-toplevel)"; cd "$ROOT"
OUT=target/perf_ap.txt; : > "$OUT"
log(){ echo "$@" | tee -a "$OUT"; }
BR=perf/array-pipeline-native-source

log "== capture edits =="
git diff -- crates/tish_compile/src/infer.rs crates/tish_compile/src/codegen.rs > target/ap.patch
[ -s target/ap.patch ] || { log "ABORT: no edit captured"; exit 1; }
git checkout -- crates/tish_compile/src/infer.rs crates/tish_compile/src/codegen.rs 2>/dev/null || true

log "== branch off fresh main =="
git checkout main --quiet && (git pull --ff-only origin main --quiet 2>/dev/null || true)
git branch -D "$BR" 2>/dev/null || true
git checkout -b "$BR" --quiet || { log "ABORT: branch"; exit 1; }
git apply target/ap.patch || { log "ABORT: patch apply"; git checkout main --quiet; exit 1; }

log "== build release =="
cargo build --release --bin tish > target/apb.log 2>&1 || { log "ABORT: release build"; tail -30 target/apb.log|sed 's/^/  /'|tee -a "$OUT"; git checkout main --quiet; exit 1; }

log "== gauntlet: array_pipeline flip + HOF/number-array regression sweep =="
bash scripts/run_perf_gauntlet.sh array_pipeline typed_array_hof array_hof fannkuch nsieve queens matmul k_nucleotide fnv_hash object_sum numeric_loop > target/apg.log 2>&1
grep -E "^[a-z_]+ +[0-9]|SOUNDNESS|SUMMARY" target/apg.log | tee -a "$OUT"
grep -q "no build/run/checksum failures" target/apg.log || { log "ABORT: SOUNDNESS FAIL (regression!)"; tail -12 target/apg.log|sed 's/^/  /'|tee -a "$OUT"; git checkout main --quiet; exit 1; }

log "== full integration =="
cargo build --bin tish > target/apbd.log 2>&1 || { log "ABORT: debug build"; tail -15 target/apbd.log|sed 's/^/  /'|tee -a "$OUT"; git checkout main --quiet; exit 1; }
cargo nextest run -p tishlang --test integration_test > target/apit.log 2>&1
if grep -qiE "test run failed| [1-9][0-9]* failed" target/apit.log; then log "ABORT: integration FAIL"; tail -10 target/apit.log|sed 's/^/  /'|tee -a "$OUT"; git checkout main --quiet; exit 1; fi
log "   integration: $(grep -E 'tests run:' target/apit.log|tail -1)"

AP_ROW=$(grep -E "^array_pipeline " target/apg.log)
log "   array_pipeline: $AP_ROW"
if ! echo "$AP_ROW" | grep -q "PASS"; then
  log "HOLD: array_pipeline still FAIL — not flipped. NOT PRing (will report the number)."
  git checkout main --quiet; log "DONE (held)"; exit 0
fi

log "== FLIP — commit + push + PR =="
git add -A
git commit -q -F - <<'MSG'
perf(native): iterate native Vec<f64> HOF sources directly — no per-pass snapshot (array_pipeline beats node)

array_pipeline (`data.filter().map().reduce()` over a 1M number array, ×10 passes) was 1.37× node:
`data` stayed a boxed `Value::Array`, so the fused HOF chain called `array_as_f64_snapshot(&data)`
EVERY pass — converting the whole 1M-element `Vec<Value>` → `Vec<f64>` ×10, even though `data` is
loop-invariant. Two changes: (1) inference now lets an array used in a fused-HOF chain
(filter/map/reduce/…) become a native `Vec<f64>` (sound: codegen boxes it for any non-fused use); (2)
`try_fused_hof_chain` iterates a native `Vec<f64>` root directly (`let __na = &data`) — a zero-copy
borrow, no snapshot. The per-pass O(n) conversion is gone. General (any fused HOF chain over a numeric
array), not fixture-specific (#317).
MSG
git push origin "$BR" >/dev/null 2>&1
{
  echo "array_pipeline (\`data.filter().map().reduce()\` over a 1M number array, ×10 passes) was 1.37× node:"
  echo "\`data\` stayed a boxed \`Value::Array\`, so the fused HOF chain called \`array_as_f64_snapshot(&data)\`"
  echo "**every pass** — converting the whole 1M-element \`Vec<Value>\` → \`Vec<f64>\` ×10, even though \`data\`"
  echo "is loop-invariant."
  echo ""
  echo "Two changes: (1) inference lets an array used in a fused-HOF chain (filter/map/reduce/…) become a"
  echo "native \`Vec<f64>\` (sound: codegen boxes it for any non-fused use); (2) \`try_fused_hof_chain\`"
  echo "iterates a native \`Vec<f64>\` root **directly** (\`let __na = &data\`) — a zero-copy borrow, no snapshot."
  echo ""
  echo "Gauntlet array_pipeline (was 172ms / 1.37× node):"
  echo '```'
  echo "$AP_ROW"
  echo '```'
  echo "Soundness: typed==boxed==node across the HOF/number-array sweep. Full integration passed. General (#317)."
} > target/pr_ap.md
url=$(gh pr create --base main --head "$BR" --title "perf(native): iterate native Vec<f64> HOF sources directly — no per-pass snapshot (array_pipeline beats node)" --body-file target/pr_ap.md 2>&1 | tail -1)
log "   PR: $url"
git checkout main --quiet
log "DONE"
