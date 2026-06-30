#!/usr/bin/env bash
# Validate + PR the loop-invariant string .length hoist (string_build O(n²)->O(n)) using the already
# captured target/strlen.patch. Lean fixture set (string_build + .length-loop controls) + full
# integration; PR only if string_build flips with no soundness failure. Output: target/perf_strlen.txt
set -uo pipefail
ROOT="$(git rev-parse --show-toplevel)"; cd "$ROOT"
OUT=target/perf_strlen.txt; : > "$OUT"
log(){ echo "$@" | tee -a "$OUT"; }
BR=perf/string-length-hoist
[ -s target/strlen.patch ] || { log "ABORT: target/strlen.patch missing/empty"; exit 1; }

log "== clean state + branch off fresh main =="
git checkout -- . 2>/dev/null || true
git checkout main --quiet && (git pull --ff-only origin main --quiet 2>/dev/null || true)
git branch -D "$BR" 2>/dev/null || true
git checkout -b "$BR" --quiet || { log "ABORT: branch"; exit 1; }
git apply target/strlen.patch || { log "ABORT: patch apply"; git checkout main --quiet; exit 1; }

log "== build release =="
cargo build --release --bin tish > target/slb.log 2>&1 || { log "ABORT: release build"; tail -30 target/slb.log|sed 's/^/  /'|tee -a "$OUT"; git checkout main --quiet; exit 1; }

log "== lean gauntlet (string_build flip + .length-loop regression controls) =="
bash scripts/run_perf_gauntlet.sh string_build fannkuch queens nsieve regex_redux k_nucleotide fnv_hash > target/slg.log 2>&1
grep -E "^[a-z_]+ +[0-9]|SOUNDNESS|SUMMARY" target/slg.log | tee -a "$OUT"
grep -q "no build/run/checksum failures" target/slg.log || { log "ABORT: SOUNDNESS FAIL (regression!)"; tail -12 target/slg.log|sed 's/^/  /'|tee -a "$OUT"; git checkout main --quiet; exit 1; }

log "== full integration =="
cargo build --bin tish > target/slbd.log 2>&1 || { log "ABORT: debug build"; tail -15 target/slbd.log|sed 's/^/  /'|tee -a "$OUT"; git checkout main --quiet; exit 1; }
cargo nextest run -p tishlang --test integration_test > target/slit.log 2>&1
if grep -qiE "test run failed| [1-9][0-9]* failed" target/slit.log; then log "ABORT: integration FAIL"; tail -10 target/slit.log|sed 's/^/  /'|tee -a "$OUT"; git checkout main --quiet; exit 1; fi
log "   integration: $(grep -E 'tests run:' target/slit.log|tail -1)"

SB_ROW=$(grep -E "^string_build " target/slg.log)
log "   string_build: $SB_ROW"
if ! echo "$SB_ROW" | grep -q "PASS"; then
  log "HOLD: string_build still FAIL — not flipped. NOT PRing (will report the number)."
  git checkout main --quiet; log "DONE (held)"; exit 0
fi

log "== FLIP — commit + push + PR =="
git add -A
git commit -q -F - <<'MSG'
perf(native): hoist loop-invariant string .length out of loop conditions (string_build beats node)

string_build was 92x slower than node: its strided `charCodeAt` checksum loops re-evaluated
`str.length` EVERY iteration — O(n²). Two causes in the #317 .length hoist:
  * `acc` (a native `String`) was never hoisted (`is_boxed` required `Value`), and `acc.length`
    emitted a full `Value::String(acc.clone())` DEEP COPY of the whole string per iteration;
  * `joined` (a boxed `Value` string) was blocked because the hoist gate bailed on ANY call in the
    body — including the read-only `joined.charCodeAt(i)`.
Fixes: (1) the gate is now base-specific — only an actual length-mutation of `base`
(push/splice/`base.length =`/`base[k] =`/delete/passing `base` to a callee/reassign) blocks the
hoist; a read-only method (charCodeAt/slice/map/…) does not. (2) native `String` receivers are
hoisted too — strings are immutable, so `.length` is always loop-invariant. Both check loops drop
from O(n²) to O(n). General (any `for (i…; i<str.length; …)` with a read-only body), not
fixture-specific (#317). #203 P0 (strings).
MSG
git push origin "$BR" >/dev/null 2>&1
{
  echo "string_build was **92× slower than node**: its strided \`charCodeAt\` checksum loops re-evaluated"
  echo "\`str.length\` **every iteration** — O(n²). Two causes in the #317 \`.length\` hoist:"
  echo "- \`acc\` (a native \`String\`) was never hoisted, and \`acc.length\` emitted a full"
  echo "  \`Value::String(acc.clone())\` **deep copy of the whole string per iteration**;"
  echo "- \`joined\` (a boxed \`Value\` string) was blocked because the gate bailed on **any** call in the"
  echo "  body — including the read-only \`joined.charCodeAt(i)\`."
  echo ""
  echo "Fixes: (1) base-specific gate — only an actual length-mutation of \`base\` blocks the hoist, not a"
  echo "read-only method; (2) native \`String\` receivers hoist too (strings are immutable). Both loops go"
  echo "O(n²) → O(n). General (any \`for (i; i<str.length; …)\` read-only body), not fixture-specific (#317)."
  echo ""
  echo "Gauntlet string_build (was 3223ms / 92× node):"
  echo '```'
  echo "$SB_ROW"
  echo '```'
  echo "Soundness: typed==boxed==node across the .length-loop controls. Full integration passed. #203 P0."
} > target/pr_strlen.md
url=$(gh pr create --base main --head "$BR" --title "perf(native): hoist loop-invariant string .length out of loop conditions (string_build beats node)" --body-file target/pr_strlen.md 2>&1 | tail -1)
log "   PR: $url"
git checkout main --quiet
log "DONE"
