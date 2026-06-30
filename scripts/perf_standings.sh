#!/usr/bin/env bash
set -uo pipefail
cd "$(git rev-parse --show-toplevel)"
git checkout main --quiet 2>/dev/null || true
git pull --ff-only origin main --quiet 2>/dev/null || true
cargo build --release --bin tish > target/st_build.log 2>&1 || { echo "BUILD FAIL"; tail -20 target/st_build.log; exit 1; }
echo "== current standings (close FAILs + controls) =="
bash scripts/run_perf_gauntlet.sh array_pipeline json_roundtrip sort_comparator k_nucleotide regex_redux fannkuch fnv_hash string_build numeric_loop 2>&1 | tail -25
