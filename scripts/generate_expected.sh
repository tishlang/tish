#!/usr/bin/env bash
# Generate tests/core/*.tish.expected from interpreter output.
# Run from repo root after building: cargo build -p tishlang&& ./scripts/generate_expected.sh

set -e
REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"
BIN="${REPO_ROOT}/target/debug/tish"
if [[ -n "$CARGO_TARGET_DIR" ]]; then
  BIN="${CARGO_TARGET_DIR}/debug/tish"
fi
if [[ ! -x "$BIN" ]]; then
  echo "Build tish first: cargo build -p tish"
  exit 1
fi
CORE="${REPO_ROOT}/tests/core"
for f in "$CORE"/*.tish; do
  [[ -f "$f" ]] || continue
  echo "Generating ${f}.expected"
  "$BIN" run "$f" --backend interp > "${f}.expected" 2>/dev/null || true
done
echo "Done. Commit tests/core/*.tish.expected as needed."
