#!/usr/bin/env bash
# Validate a perf branch end-to-end and open its PR — ONE command instead of ~10 piecemeal ones.
#
#   scripts/perf_pr.sh <branch> <fixture> "<pr title>" [body-file] [--integration]
#
# Gates the PR on the gauntlet soundness line (typed==boxed==node, no checksum failures).
# With --integration it also runs the full corpus locally before pushing (use for GENERAL codegen
# changes; isolated ones can rely on CI). Builds use the shared sccache cache, so they're fast.
set -uo pipefail
BRANCH="${1:?usage: perf_pr.sh <branch> <fixture> <title> [body-file] [--integration]}"
FIXTURE="${2:?missing fixture}"
TITLE="${3:?missing title}"
BODY=""; INTEG=0
for a in "${@:4}"; do
  case "$a" in
    --integration) INTEG=1 ;;
    *) BODY="$a" ;;
  esac
done
ROOT="$(git rev-parse --show-toplevel)"; cd "$ROOT"

echo "===== perf_pr: $BRANCH ($FIXTURE) ====="
git fetch origin --quiet 2>/dev/null
echo "-- commits on origin/main..$BRANCH --"
git log --oneline "origin/main..$BRANCH" 2>/dev/null || true
# If the branch lives in an agent worktree, free it (work is committed on the branch) so we can
# check it out here.
WT=$(git worktree list --porcelain | awk -v b="refs/heads/$BRANCH" '/^worktree /{p=$2} /^branch /{if($2==b) print p}')
if [ -n "$WT" ] && [ "$WT" != "$ROOT" ]; then
  echo "-- freeing worktree $WT --"
  git worktree remove -f -f "$WT" 2>/dev/null; git worktree prune 2>/dev/null
fi
git checkout "$BRANCH" --quiet || { echo "ABORT: checkout failed"; exit 1; }

echo "-- build (sccache) --"
cargo build --release --bin tish 2>&1 | tail -1
[ -x target/release/tish ] || { echo "ABORT: build failed"; exit 1; }

echo "-- gauntlet soundness + perf ($FIXTURE) --"
GLOG="target/perf_pr_gauntlet.log"
TMPDIR="$ROOT/target/gt" bash scripts/run_perf_gauntlet.sh "$FIXTURE" >"$GLOG" 2>&1
tail -8 "$GLOG"
grep -q "no build/run/checksum failures" "$GLOG" || { echo "ABORT: gauntlet soundness FAILED"; exit 1; }

if [ "$INTEG" = 1 ]; then
  echo "-- full integration corpus --"
  ILOG="target/perf_pr_integ.log"
  cargo nextest run -p tishlang --test integration_test >"$ILOG" 2>&1
  tail -3 "$ILOG"
  if grep -qiE "test run failed| [1-9][0-9]* failed" "$ILOG"; then echo "ABORT: integration FAILED"; exit 1; fi
  grep -q "tests run:" "$ILOG" || { echo "ABORT: integration did not run"; exit 1; }
fi

echo "-- push + open PR --"
git push origin "$BRANCH" 2>&1 | tail -2
if [ -n "$BODY" ] && [ -f "$BODY" ]; then
  gh pr create --base main --head "$BRANCH" --title "$TITLE" --body-file "$BODY" 2>&1 | tail -2
else
  gh pr create --base main --head "$BRANCH" --title "$TITLE" --body "$TITLE" 2>&1 | tail -2
fi
git checkout main --quiet 2>/dev/null || true
echo "===== perf_pr done ====="
