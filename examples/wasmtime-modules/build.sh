#!/bin/bash
set -e
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"
# Use workspace tish when run from tish/examples/wasmtime-modules (has wasi support)
TISH_MF="../../Cargo.toml"
if [ -f "$TISH_MF" ]; then
  TISH="cargo run -p tishlang--manifest-path $TISH_MF --"
else
  TISH="tish"
fi
DIST=dist
mkdir -p $DIST
$TISH build src/math.tish -o $DIST/math --target wasi
$TISH build src/greet.tish -o $DIST/greet --target wasi
$TISH build src/main.tish -o $DIST/main --target wasi
echo "Built: $DIST/main.wasm, $DIST/math.wasm, $DIST/greet.wasm"
