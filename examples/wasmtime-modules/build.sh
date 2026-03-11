#!/bin/bash
set -e
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"
# Use workspace tish when run from tish/examples/wasmtime-modules (has wasi support)
TISH_MF="../../Cargo.toml"
if [ -f "$TISH_MF" ]; then
  TISH="cargo run -p tish --manifest-path $TISH_MF --"
else
  TISH="tish"
fi
DIST=dist
mkdir -p $DIST
$TISH compile src/math.tish -o $DIST/math --target wasi
$TISH compile src/greet.tish -o $DIST/greet --target wasi
$TISH compile src/main.tish -o $DIST/main --target wasi
echo "Built: $DIST/main.wasm, $DIST/math.wasm, $DIST/greet.wasm"
