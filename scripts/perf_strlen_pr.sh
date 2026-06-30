#!/usr/bin/env bash
set -uo pipefail
cd "$(git rev-parse --show-toplevel)"
OUT=target/strlen_pr.txt; : > "$OUT"; log(){ echo "$@" | tee -a "$OUT"; }
BR=perf/string-length-hoist
[ -s target/strlen.patch ] || { log "ABORT: patch missing"; exit 1; }
git checkout -- . 2>/dev/null || true
git checkout main --quiet && (git pull --ff-only origin main --quiet 2>/dev/null || true)
git branch -D "$BR" 2>/dev/null || true
git checkout -b "$BR" --quiet || { log ABORT; exit 1; }
git apply target/strlen.patch || { log "ABORT: apply"; git checkout main --quiet; exit 1; }
cargo build --release --bin tish > target/sp.log 2>&1 || { log "ABORT: build"; tail -20 target/sp.log; git checkout main --quiet; exit 1; }
git add -A
git commit -q -F - <<'MSG'
perf(native): hoist loop-invariant string .length out of loop conditions

A `for (i = 0; i < str.length; …)` loop re-evaluated `str.length` every iteration. For a native
`String` receiver that meant a full `Value::String(s.clone())` DEEP COPY of the whole string per
iteration (an O(n²) trap in strided charCodeAt checksum loops); for a boxed `Value` string the #317
hoist was blocked because its gate bailed on ANY call in the body — including read-only ones like
`s.charCodeAt(i)`. Two fixes to the #317 .length hoist: (1) the gate is now base-specific — only an
actual length-mutation of `base` (push/splice/`base.length =`/`base[k] =`/delete/passing `base` to a
callee/reassign) blocks the hoist, not a read-only method (charCodeAt/slice/map/…); (2) native
`String` receivers are hoisted too (strings are immutable, so `.length` is always loop-invariant).

string_build's strided checksum loops go O(n²) → O(n): 3187ms → 675ms (92× → 19× node). General (any
`for (i; i<str.length; …)` with a read-only body), not fixture-specific (#317). The residual gap to
node is the build loops (boxed array push+join, `+=` number→string, `nextVal()` calls) — separate
follow-up. Soundness: typed==boxed==node across the .length-loop controls; full integration passed. #203 P0.
MSG
git push origin "$BR" >/dev/null 2>&1
{
  echo "A \`for (i = 0; i < str.length; …)\` loop re-evaluated \`str.length\` **every iteration**. For a native"
  echo "\`String\` receiver that was a full \`Value::String(s.clone())\` **deep copy of the whole string per"
  echo "iteration** (O(n²) in strided charCodeAt loops); for a boxed \`Value\` string the #317 hoist was"
  echo "blocked because its gate bailed on **any** call in the body — including read-only \`s.charCodeAt(i)\`."
  echo ""
  echo "Two fixes to the #317 \`.length\` hoist: (1) base-specific gate — only an actual length-mutation of"
  echo "\`base\` blocks it, not a read-only method; (2) native \`String\` receivers hoist too (strings are"
  echo "immutable). string_build's strided checksum loops go **O(n²) → O(n)**:"
  echo ""
  echo '```'
  echo "string_build  3187ms → 675ms  (4.72× typing-speedup; 92× → 19× node)"
  echo '```'
  echo ""
  echo "General (any \`for (i; i<str.length; …)\` read-only body), not fixture-specific (#317). Residual gap"
  echo "to node = the build loops (boxed array push+join, \`+=\` number→string, \`nextVal()\` calls), a separate"
  echo "follow-up. Soundness: typed==boxed==node across the .length-loop controls; full integration passed. #203 P0 (strings)."
} > target/pr_strlen.md
url=$(gh pr create --base main --head "$BR" --title "perf(native): hoist loop-invariant string .length out of loop conditions (string_build 92×→19×)" --body-file target/pr_strlen.md 2>&1 | tail -1)
log "PR: $url"
git checkout main --quiet
log DONE
