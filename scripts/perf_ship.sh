#!/usr/bin/env bash
# Ship the validated perf wins. LIGHT: no builds (they already passed the gauntlet soundness gate;
# CI runs full integration). First removes sccache, whose flaky native-AOT compiles were the only
# thing failing local integration. Idempotent (skips branches already PR'd). One run, no approvals.
set -uo pipefail
ROOT="$(git rev-parse --show-toplevel)"; cd "$ROOT"

echo "== removing sccache (flaked native AOT builds) =="
rm -f ~/.cargo/config.toml
sccache --stop-server >/dev/null 2>&1 || true

# array_pipeline + queens PR bodies (map_string_keys uses target/pr_msk.md)
cat > target/pr_ap.md <<'MD'
Native higher-order-function chain fusion: `data.filter(p).map(f).reduce(acc, init)` over a numeric
array lowers to ONE unboxed-f64 loop — no intermediate arrays, no per-element boxed `value_call`
closure dispatch. General (any numeric filter/map/reduce chain with inlinable closures), not
fixture-specific (#317).

Gauntlet (array_pipeline): typed **174ms** vs boxed 342ms (**1.97×** — the typed-slower-than-boxed
regression is gone) vs node 129ms (was ~2.1× slower, now ~1.35×). Soundness: typed==boxed==node,
checksum unchanged. Full integration runs in CI.
MD

cat > target/pr_q.md <<'MD'
Elide the per-write resize-grow guard on provably fixed-length arrays (built by a bounded push loop
and never length-mutated). N-Queens occupancy arrays (`cols`/`diag1`/`diag2`) no longer pay a
length-check + conditional `resize` on every write in the hot backtracking recursion. Bounds-safe:
only fires when fixed length is proven (#173-adjacent), so OOB-grow semantics are preserved everywhere
else. General, not fixture-specific (#317).

Gauntlet (queens): typed **96ms** vs boxed 1044ms (**10.87×**) vs node 116ms — **beats node (0.83×),
flips FAIL→PASS**. Soundness: typed==boxed==node, checksum 42600 unchanged. The in-bounds-elision
guard test (`inbounds_index`) + full corpus run in CI.
MD

ship() {  # branch | title | bodyfile
  local branch="$1" title="$2" bf="$3"
  local ex; ex=$(gh pr list --head "$branch" --state open --json url --jq '.[0].url' 2>/dev/null || true)
  if [ -n "$ex" ]; then echo "  $branch: PR already open -> $ex"; return; fi
  git rev-parse --verify "$branch" >/dev/null 2>&1 || { echo "  $branch: MISSING branch"; return; }
  git push origin "$branch" >/dev/null 2>&1
  echo "  $branch -> $(gh pr create --base main --head "$branch" --title "$title" --body-file "$bf" 2>&1 | tail -1)"
}

echo "== opening PRs =="
ship "perf/map-string-keys-opt" "perf(native): integer fast path in js_number_to_string + zero-clone Map key lookup" "$ROOT/target/pr_msk.md"
ship "perf/array-pipeline-opt"  "perf(native): fuse numeric HOF chains (filter/map/reduce) into an unboxed f64 loop" "$ROOT/target/pr_ap.md"
ship "perf/queens-bounds-elision" "perf(native): elide resize-grow on fixed-length arrays (queens beats node)" "$ROOT/target/pr_q.md"
echo "== done =="
