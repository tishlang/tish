#!/usr/bin/env bash
# One-shot snapshot of the perf-iteration environment (machine, cache, branches, agents).
# Usage: scripts/perf_status.sh
set -uo pipefail
cd "$(git rev-parse --show-toplevel)"

echo "== load (1/5/15m, $(sysctl -n hw.ncpu 2>/dev/null || echo ?) cores) =="
uptime | sed 's/.*load/load/'
echo "== dune-ide vscode-host (leak source; should be ~1-2, 0=clean) =="
pgrep -f "dune-ide.*vscode-host" 2>/dev/null | wc -l | tr -d ' '
echo "== top CPU =="
ps aux | sort -nrk3 | head -5 | awk '{printf "  %5s%% %.55s\n",$3,$11" "$12}'
echo "== sccache =="
sccache --show-stats 2>/dev/null | grep -iE "^Cache hits |^Cache misses |hits rate +[0-9]|^Compile requests" | sed 's/^/  /' | head -4
echo "== perf/* branches ahead of origin/main =="
git fetch origin --quiet 2>/dev/null
for b in $(git branch --list 'perf/*' --format='%(refname:short)' 2>/dev/null); do
  n=$(git log --oneline "origin/main..$b" 2>/dev/null | wc -l | tr -d ' ')
  [ "${n:-0}" -gt 0 ] && echo "  $b: $n commit(s) — $(git log -1 --format=%s "$b" 2>/dev/null | head -c 64)"
done
echo "== agent worktrees =="
git worktree list 2>/dev/null | grep "worktrees/agent-" | sed 's/^/  /' || echo "  none"
echo "== running rustc/cargo =="
echo "  $(pgrep -c -E 'rustc|cargo' 2>/dev/null || echo 0) process(es)"
echo "== open PRs =="
gh pr list --state open --json number,title --jq '.[]|"  #\(.number) \(.title)"' 2>/dev/null | head -10 || echo "  (gh unavailable)"
