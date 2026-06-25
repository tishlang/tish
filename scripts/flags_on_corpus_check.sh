#!/usr/bin/env bash
# Native typed-codegen SOUNDNESS sweep: build every tests/core/*.tish twice on the rust native
# backend — once with the optimizations ON (the default) and once OFF (TISH_NATIVE_OPT=0, the dynamic
# Value baseline) — run both, and diff their output. The optimizations must be behavior-preserving,
# so optimized output MUST equal boxed output. This is the soundness differential; it does NOT use
# the committed .expected (which bakes in run-specific `<N>ms` timings and is unreliable for diffing).
# Inherent non-determinism (Date.now timing) is masked: any `<digits>ms` token is normalized before
# the diff. A fixture whose two builds differ is a real miscompile in the typed codegen.
set -u
cd "$(dirname "$0")/.." || exit 1
TISH="${TISH_BIN:-target/release/tish}"
fail=0; n=0; pass=0
tmp=$(mktemp -d)
# Normalize inherent non-determinism (timings) so only real divergence shows.
norm() { sed -E 's/[0-9]+(\.[0-9]+)?ms/Nms/g'; }
for src in tests/core/*.tish; do
  n=$((n+1))
  base=$(basename "${src%.tish}")
  on="$tmp/${base}_on"; off="$tmp/${base}_off"
  if ! env TISH_FAST_NATIVE_BUILD=1 "$TISH" build "$src" -o "$on" >/dev/null 2>"$tmp/err_on"; then
    echo "BUILD-FAIL(opt)  $src"; sed 's/^/    /' "$tmp/err_on" | head -4; fail=$((fail+1)); continue
  fi
  if ! env TISH_FAST_NATIVE_BUILD=1 TISH_NATIVE_OPT=0 "$TISH" build "$src" -o "$off" >/dev/null 2>"$tmp/err_off"; then
    echo "BUILD-FAIL(box)  $src"; sed 's/^/    /' "$tmp/err_off" | head -4; fail=$((fail+1)); continue
  fi
  got=$("$on" 2>/dev/null | norm)
  want=$("$off" 2>/dev/null | norm)
  if [ "$got" = "$want" ]; then pass=$((pass+1)); else
    echo "DIFF             $src"; diff <(printf '%s' "$want") <(printf '%s' "$got") | head -8; fail=$((fail+1))
  fi
done
rm -rf "$tmp"
echo "native-opt soundness: $pass/$n typed==boxed, $fail fail"
[ "$fail" -eq 0 ]
