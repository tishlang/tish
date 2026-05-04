#!/usr/bin/env bash
# tishlang depends on `tish-pg` at ../../../tish-pg from crates/tish → sibling of this workspace root.
# CI only checks out the `tish` repo; clone the shim here so `cargo build -p tishlang --features full` works.
set -euo pipefail
if [[ -n "${GITHUB_WORKSPACE:-}" ]]; then
  root="${GITHUB_WORKSPACE}"
else
  root="$(git rev-parse --show-toplevel)"
fi
dest="$(dirname "${root}")/tish-pg"
if [[ -f "${dest}/Cargo.toml" ]]; then
  echo "tish-pg already present at ${dest}"
  exit 0
fi
echo "Cloning tish-pg into ${dest}"
mkdir -p "$(dirname "${dest}")"
git clone --depth 1 https://github.com/tishlang/tish-pg.git "${dest}"
