#!/usr/bin/env bash
# Merge the ready PRs (now, if CI is green; else enable auto-merge so GitHub merges when it goes
# green) and then print fresh full-gauntlet standings to pick the next targets. One run, no approvals.
set -uo pipefail
ROOT="$(git rev-parse --show-toplevel)"; cd "$ROOT"
OUT="$ROOT/target/perf_merge_status.txt"; : > "$OUT"
log(){ echo "$@" | tee -a "$OUT"; }

log "== merge ready PRs =="
for pr in 346 347 348; do
  out=$(gh pr merge "$pr" --squash --delete-branch 2>&1)
  if echo "$out" | grep -qiE "checks|pending|not mergeable|required|in progress|expected"; then
    out=$(gh pr merge "$pr" --squash --delete-branch --auto 2>&1)
  fi
  log "  #$pr: $(echo "$out" | tail -1)"
done

log ""
log "== refresh main =="
git checkout main --quiet 2>/dev/null || true
git pull --ff-only origin main --quiet 2>/dev/null || true
cargo build --release --bin tish 2>&1 | tail -1 | sed 's/^/  /' | tee -a "$OUT"

log ""
log "== full gauntlet standings (main) =="
bash scripts/run_perf_gauntlet.sh 2>&1 | tail -40 | tee -a "$OUT"
log ""
log "== DONE =="
