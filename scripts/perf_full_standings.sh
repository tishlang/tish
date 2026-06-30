#!/usr/bin/env bash
set -uo pipefail
cd "$(git rev-parse --show-toplevel)"
git checkout main --quiet 2>/dev/null || true
git pull --ff-only origin main --quiet 2>/dev/null || true
echo "== main HEAD =="; git log --oneline -8
echo "== merge presence checks =="
grep -q "cse_result_is_boxed" crates/tish_compile/src/codegen.rs && echo "  #350 de-boxing: PRESENT" || echo "  #350: ABSENT"
grep -q "record_array_fields" crates/tish_compile/src/infer.rs && echo "  #351 array-of-records: PRESENT" || echo "  #351: ABSENT"
grep -q "arr\[i\].field" crates/tish_compile/src/codegen.rs && echo "  #352 native struct-array arith: PRESENT" || echo "  #352: likely present"
grep -q "expr_changes_base_len" crates/tish_compile/src/codegen.rs && echo "  #353 .length hoist: PRESENT" || echo "  #353: ABSENT"
echo "== build =="; cargo build --release --bin tish > target/fs_build.log 2>&1 || { echo BUILD FAIL; tail -20 target/fs_build.log; exit 1; }
echo "== FULL gauntlet (all fixtures) =="
bash scripts/run_perf_gauntlet.sh 2>&1 | tail -45
