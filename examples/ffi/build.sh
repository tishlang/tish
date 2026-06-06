#!/usr/bin/env bash
# Build the example native extension and stage it as `mathext.lib` (an extension-neutral name
# that dlopen accepts on macOS/Linux/Windows alike, so the demo path is portable).
set -euo pipefail
cd "$(dirname "$0")"

( cd mathext && cargo build --release )

lib=""
for cand in libmathext.dylib libmathext.so mathext.dll; do
  if [[ -f "mathext/target/release/$cand" ]]; then
    lib="mathext/target/release/$cand"
    break
  fi
done

if [[ -z "$lib" ]]; then
  echo "error: could not find the built cdylib under mathext/target/release/" >&2
  exit 1
fi

cp "$lib" mathext.lib
echo "Built $(basename "$lib") -> examples/ffi/mathext.lib"
echo "Run:  tish run examples/ffi/demo.tish"
