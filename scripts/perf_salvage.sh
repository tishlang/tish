#!/usr/bin/env bash
# After an agent/process crash: show committed + uncommitted work in every agent worktree, so nothing
# is silently lost. Usage: scripts/perf_salvage.sh
set -uo pipefail
cd "$(git rev-parse --show-toplevel)"

echo "== perf/* branches ahead of origin/main =="
git fetch origin --quiet 2>/dev/null
for b in $(git branch --list 'perf/*' --format='%(refname:short)' 2>/dev/null); do
  n=$(git log --oneline "origin/main..$b" 2>/dev/null | wc -l | tr -d ' ')
  echo "  $b: ${n:-0} commit(s)"
done

echo "== agent worktrees (HEAD + any uncommitted work) =="
git worktree list 2>/dev/null | grep "worktrees/agent-" | awk '{print $1}' | while read -r path; do
  echo "--- $path ---"
  echo "    HEAD: $(git -C "$path" log -1 --oneline 2>/dev/null)"
  u=$(git -C "$path" status --short 2>/dev/null | head -6)
  if [ -n "$u" ]; then
    echo "    UNCOMMITTED:"
    echo "$u" | sed 's/^/      /'
    git -C "$path" diff --stat 2>/dev/null | tail -3 | sed 's/^/      /'
  else
    echo "    (clean working tree)"
  fi
done
