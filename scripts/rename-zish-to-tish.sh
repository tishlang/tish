#!/usr/bin/env bash
# Recursively rename *.zish -> *.tish
# Usage: ./scripts/rename-zish-to-tish.sh [directory]
set -euo pipefail
DIR="${1:-.}"
find "$DIR" -name '*.zish' -type f -print0 | while IFS= read -r -d '' f; do
  mv "$f" "${f%.zish}.tish"
done
