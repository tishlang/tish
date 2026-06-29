#!/usr/bin/env bash
# Read-only: dump the boxed-Value representation + every hot access/construction site into one file
# so the design can proceed without further per-command approvals. Output: target/value_repr.txt
set -uo pipefail
cd "$(git rev-parse --show-toplevel)"
OUT=target/value_repr.txt
exec > "$OUT" 2>&1

echo "############ Value enum (value.rs) ############"
grep -n -A70 "^pub enum Value" crates/tish_core/src/value.rs

echo ""; echo "############ ObjectData struct ############"
grep -n -A45 "pub struct ObjectData" crates/tish_core/src/value.rs

echo ""; echo "############ PropMap (struct/type + impl) ############"
grep -n -A60 "struct PropMap\|type PropMap" crates/tish_core/src/value.rs
echo "--- PropMap methods ---"
grep -n -A8 "impl PropMap" crates/tish_core/src/value.rs

echo ""; echo "############ Object/Array container types in Value ############"
grep -n "Object(\|Array(\|VmRef<\|ObjectData\|PropMap\|ArcStr\|String(" crates/tish_core/src/value.rs | head -40

echo ""; echo "############ runtime get_prop / set_prop / get_index / PropIC ############"
grep -n -A35 "pub fn get_prop\b\|pub fn set_prop\b\|pub fn get_prop_ic\|pub struct PropIC" crates/tish_runtime/src/lib.rs

echo ""; echo "############ runtime get_index / set_index / array element access ############"
grep -n -A25 "pub fn get_index\b\|pub fn set_index\b\|pub fn vm_read\b\|pub fn index_get" crates/tish_runtime/src/lib.rs

echo ""; echo "############ object construction + spread (merge_from, object_literal) ############"
grep -n -A18 "pub fn merge_from\|pub fn object\b\|fn build_object\|object_spread\|pub fn new_object" crates/tish_core/src/value.rs crates/tish_runtime/src/lib.rs

echo ""; echo "############ codegen: how member access / object literals are emitted ############"
grep -n "get_prop_ic\|get_prop(\|PropIC\|emit_object\|object literal\|MemberExpr\|PropMap::\|ObjectData" crates/tish_compile/src/codegen.rs | head -50

echo ""; echo "############ PropMap definition file confirm + ArcStr ############"
grep -rn "pub type PropMap\|pub struct PropMap\|pub use.*PropMap\|pub type ArcStr\|ObjectData {" crates/tish_core/src/ crates/tish_runtime/src/ | head

echo "DONE"
