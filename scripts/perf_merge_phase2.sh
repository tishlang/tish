#!/usr/bin/env bash
# Merge #349/#350 (now if green, else auto-merge), then dump the inference scaffolding needed to design
# the untyped->struct inference (Phase 2 of the array_records lever). Output: target/merge_phase2.txt
set -uo pipefail
cd "$(git rev-parse --show-toplevel)"
OUT=target/merge_phase2.txt; : > "$OUT"
log(){ echo "$@" | tee -a "$OUT"; }

log "== merge #349 #350 =="
for pr in 349 350; do
  out=$(gh pr merge "$pr" --squash --delete-branch 2>&1)
  if echo "$out" | grep -qiE "checks|pending|not mergeable|required|in progress|expected"; then
    out=$(gh pr merge "$pr" --squash --delete-branch --auto 2>&1)
  fi
  log "  #$pr: $(echo "$out" | tail -1)"
done

log ""
log "== Phase 2 scaffolding dump =="
{
echo "#### infer.rs: passes, local/array inference, struct synthesis, alias registration ####"
grep -nE "pub fn infer|fn infer_|aggregate_infer|RustType::Named|synthesi|register.*alias|type_alias|aliases|Object\(|fn [a-z_]*program|fn analyze_aggregate|detect_rec_struct" crates/tish_compile/src/infer.rs | head -70
echo ""
echo "#### codegen: type_context population, how a let-decl / array literal gets its type ####"
grep -nE "type_context|set_type|get_type|register.*alias|alias|struct TypeContext|Expr::Array|VarDecl|native_vec_init_type|infer.*init|Vec\(Box::new\(RustType" crates/tish_compile/src/codegen.rs | head -50
echo ""
echo "#### where the program is type-inferred before codegen (entry points) ####"
grep -rnE "infer_program|aggregate_infer_program|run_inference|fn compile|inference|register_type_aliases|collect.*alias" crates/tish_compile/src/lib.rs crates/tish_compile/src/infer.rs | head -30
} >> "$OUT" 2>&1
log "== DONE =="
