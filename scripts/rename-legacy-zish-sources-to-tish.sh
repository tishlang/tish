#!/usr/bin/env bash
# One-time / bulk migration: rename legacy *.zish sources to *.tish.
# Usage: ./scripts/rename-legacy-zish-sources-to-tish.sh [directory]
set -euo pipefail
DIR="${1:-.}"
find "$DIR" -name '*.zish' -type f -print0 | while IFS= read -r -d '' f; do
  mv "$f" "${f%.zish}.tish"
done
