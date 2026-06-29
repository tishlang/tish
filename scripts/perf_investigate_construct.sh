#!/usr/bin/env bash
# Read-only: dump the object-literal->struct construction path so the Phase-1 fix can be designed
# without further approvals. Output: target/construct_repr.txt
set -uo pipefail
cd "$(git rev-parse --show-toplevel)"
OUT=target/construct_repr.txt
exec > "$OUT" 2>&1

echo "############ from_value_expr (types.rs) — the Named arm generates get_prop extraction ############"
grep -n "fn from_value_expr\|RustType::Named\|fn default_value\|get_prop\|fn to_value_expr" crates/tish_compile/src/types.rs | head -40
echo "--- from_value_expr body ---"
awk '/fn from_value_expr/{f=1} f{print NR": "$0} /^    }/{if(f)c++; if(c>=1 && f && /^    }/){f=0}}' crates/tish_compile/src/types.rs | head -120

echo ""; echo "############ array .push emission + typed-vec method routing (codegen.rs) ############"
grep -n '"push"\|array_push\|\.push(\|push_typed\|fn emit_method\|Vec(\|native_vec\|elem_type\|element_type\|inner_type' crates/tish_compile/src/codegen.rs | head -60

echo ""; echo "############ emit_native_expr signature + struct-literal fast path region ############"
grep -n "fn emit_native_expr\|fn emit_typed_expr\|RustType::Named { name, fields }\|named_struct_ident\|from_value_expr" crates/tish_compile/src/codegen.rs | head -40

echo "DONE"
