#!/usr/bin/env bash
# Optimized-native correctness sweep: build every tests/core/*.tish on the rust native backend with
# the native typed-codegen optimizations ON (the default now), run it, and diff stdout against the
# committed .expected. The optimizations must be behavior-preserving (only faster), so the output
# must equal the reference. The boxed baseline is the same build with `TISH_NATIVE_OPT=0`.
set -u
cd "$(dirname "$0")/.." || exit 1
TISH="${TISH_BIN:-target/release/tish}"
fail=0; n=0; pass=0
tmp=$(mktemp -d)
for src in tests/core/*.tish; do
  exp="${src}.expected"
  [ -f "$exp" ] || continue
  n=$((n+1))
  bin="$tmp/$(basename "${src%.tish}")"
  if ! env TISH_FAST_NATIVE_BUILD=1 "$TISH" build "$src" -o "$bin" >/dev/null 2>"$tmp/err"; then
    echo "BUILD-FAIL  $src"; sed 's/^/    /' "$tmp/err" | head -4; fail=$((fail+1)); continue
  fi
  got=$("$bin" 2>/dev/null)
  want=$(cat "$exp")
  if [ "$got" = "$want" ]; then pass=$((pass+1)); else
    echo "DIFF        $src"; diff <(printf '%s' "$want") <(printf '%s' "$got") | head -8; fail=$((fail+1))
  fi
done
rm -rf "$tmp"
echo "flags-on corpus: $pass/$n match, $fail fail"
[ "$fail" -eq 0 ]
