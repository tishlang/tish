#!/usr/bin/env bash
# Flags-ON correctness sweep: build every tests/core/*.tish on the rust native backend with ALL
# typed-native flags set, run it, and diff stdout against the committed .expected. The dark-ship
# flags must be behavior-preserving (only faster), so flags-on output must equal the reference.
# This complements test_mvp_programs_native (which runs flags-OFF) by exercising the typed codegen.
set -u
cd "$(dirname "$0")/.." || exit 1
TISH="${TISH_BIN:-target/release/tish}"
FLAGS=(TISH_PARAM_NATIVE=1 TISH_PARAM_INFER=1 TISH_NATIVE_FN=1 TISH_STRUCT_INFER=1 TISH_FUSED_HOF=1 TISH_NATIVE_HOF=1)
fail=0; n=0; pass=0
tmp=$(mktemp -d)
for src in tests/core/*.tish; do
  exp="${src}.expected"
  [ -f "$exp" ] || continue
  n=$((n+1))
  bin="$tmp/$(basename "${src%.tish}")"
  if ! env "${FLAGS[@]}" TISH_FAST_NATIVE_BUILD=1 "$TISH" build "$src" -o "$bin" >/dev/null 2>"$tmp/err"; then
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
