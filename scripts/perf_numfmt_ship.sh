#!/usr/bin/env bash
# Validate + PR the allocation-free js_number_to_string_into float path. number->string is used
# everywhere, so soundness is paramount: full integration (number_to_string/json/parity tests) +
# gauntlet soundness on number-heavy fixtures. PR if json_roundtrip improves with all green.
# Output: target/perf_numfmt.txt
set -uo pipefail
ROOT="$(git rev-parse --show-toplevel)"; cd "$ROOT"
OUT=target/perf_numfmt.txt; : > "$OUT"
log(){ echo "$@" | tee -a "$OUT"; }
BR=perf/numfmt-no-alloc

log "== capture edit =="
git diff -- crates/tish_core/src/value.rs > target/numfmt.patch
[ -s target/numfmt.patch ] || { log "ABORT: no edit captured"; exit 1; }
git checkout -- crates/tish_core/src/value.rs 2>/dev/null || true

log "== branch off fresh main =="
git checkout main --quiet && (git pull --ff-only origin main --quiet 2>/dev/null || true)
git branch -D "$BR" 2>/dev/null || true
git checkout -b "$BR" --quiet || { log "ABORT: branch"; exit 1; }
git apply target/numfmt.patch || { log "ABORT: patch apply"; git checkout main --quiet; exit 1; }

log "== build release =="
cargo build --release --bin tish > target/nfb.log 2>&1 || { log "ABORT: release build"; tail -30 target/nfb.log|sed 's/^/  /'|tee -a "$OUT"; git checkout main --quiet; exit 1; }

log "== gauntlet: json_roundtrip + number-formatting soundness sweep =="
bash scripts/run_perf_gauntlet.sh json_roundtrip fasta mandelbrot nbody math_trig map_string_keys string_concat numeric_loop > target/nfg.log 2>&1
grep -E "^[a-z_]+ +[0-9]|SOUNDNESS|SUMMARY" target/nfg.log | tee -a "$OUT"
grep -q "no build/run/checksum failures" target/nfg.log || { log "ABORT: SOUNDNESS FAIL (number formatting diverged!)"; tail -12 target/nfg.log|sed 's/^/  /'|tee -a "$OUT"; git checkout main --quiet; exit 1; }

log "== full integration (number_to_string / json / parity tests) =="
cargo build --bin tish > target/nfbd.log 2>&1 || { log "ABORT: debug build"; tail -15 target/nfbd.log|sed 's/^/  /'|tee -a "$OUT"; git checkout main --quiet; exit 1; }
cargo nextest run -p tishlang --test integration_test > target/nfit.log 2>&1
if grep -qiE "test run failed| [1-9][0-9]* failed" target/nfit.log; then log "ABORT: integration FAIL"; tail -12 target/nfit.log|sed 's/^/  /'|tee -a "$OUT"; git checkout main --quiet; exit 1; fi
log "   integration: $(grep -E 'tests run:' target/nfit.log|tail -1)"

JR_ROW=$(grep -E "^json_roundtrip " target/nfg.log)
log "   json_roundtrip: $JR_ROW"
JR_T=$(echo "$JR_ROW" | awk '{print $3}' | tr -d 'ms')
if [ -z "$JR_T" ] || ! [ "${JR_T%%.*}" -lt 155 ] 2>/dev/null; then
  log "HOLD: json_roundtrip not improved (>=155ms or unparsed); NOT PRing (number-fmt change had no measurable effect here)."
  git checkout main --quiet; log "DONE (held)"; exit 0
fi

FLIP=""; echo "$JR_ROW" | grep -q "PASS" && FLIP=" — flips json_roundtrip to PASS (beats node)"
log "== WIN${FLIP} — commit + push + PR =="
git add -A
git commit -q -F - <<'MSG'
perf(core): allocation-free float formatting in js_number_to_string_into

`js_number_to_string_into` (the ECMAScript Number::toString primitive behind JSON.stringify, template
literals, String(n), console.log, and `+=`) made 3-4 heap allocations on every non-integer float:
`format!("{:e}")`, the `digits` char-collect, `"0".repeat()`, and `e.to_string()`. Rewritten to a
reused thread-local `{:e}` buffer + a stack digit buffer + `itoa` for the exponent — ZERO allocations
per call. Output is byte-identical (same `{:e}` shortest-round-trip digits, same ECMAScript point/k
formatting). Broad real-world win (every number serialized to JSON / interpolated / logged), and the
JSON serialization hot path in particular. #203 P0 (JSON/strings).
MSG
git push origin "$BR" >/dev/null 2>&1
{
  echo "\`js_number_to_string_into\` — the ECMAScript \`Number::toString\` primitive behind \`JSON.stringify\`,"
  echo "template literals, \`String(n)\`, \`console.log\`, and \`+=\` — made **3-4 heap allocations on every"
  echo "non-integer float** (\`format!(\"{:e}\")\`, the \`digits\` char-collect, \`\"0\".repeat()\`, \`e.to_string()\`)."
  echo ""
  echo "Rewritten to a reused thread-local \`{:e}\` buffer + a stack digit buffer + \`itoa\` for the exponent —"
  echo "**zero allocations per call**. Output is byte-identical (same \`{:e}\` shortest-round-trip digits, same"
  echo "ECMAScript point/k formatting). Broad real-world win — every number serialized to JSON / interpolated"
  echo "/ logged."
  echo ""
  echo "Gauntlet json_roundtrip (was 157ms / 1.21× node):"
  echo '```'
  echo "$JR_ROW"
  echo '```'
  echo "Soundness: typed==boxed==node across the number-formatting sweep (fasta/mandelbrot/nbody/math_trig/…);"
  echo "full integration (number_to_string / json / parity tests) passed. #203 P0."
} > target/pr_numfmt.md
url=$(gh pr create --base main --head "$BR" --title "perf(core): allocation-free float formatting in js_number_to_string_into (faster JSON/number serialization)" --body-file target/pr_numfmt.md 2>&1 | tail -1)
log "   PR: $url"
git checkout main --quiet
log "DONE"
