//! GBA static-type-aware struct reflection. On `--target gba`, `Object.keys/values/entries(s)` and
//! object spread `{ ...s }` over a statically-known struct (`RustType::Named`) lower to NATIVE reads of
//! the `TishStruct_*` Rust struct — the struct is never boxed to a `Value::Struct` and no intermediate
//! hashmap `Value::Object` is built. Only the RESULT (an array / the spread object) is a `Value`.
//!
//! DESKTOP is deliberately untouched: reflection there keeps the boxed path (a struct materialises to a
//! real `Value::Object`, already correct), and struct spread boxes exactly as before — so the hot
//! `object_spread` bench keeps its clone-shape path. Asserted below so a future change that leaks the
//! GBA native lowering onto desktop (bench regression) fails here.

use std::path::PathBuf;

use tishlang_compile::{compile_project_full, compile_project_full_emit, NativeEmitMode};

fn emit(src: &str, name: &str, gba: bool) -> String {
    let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    std::fs::write(&path, src).unwrap();
    if gba {
        compile_project_full_emit(&path, path.parent(), &[], true, NativeEmitMode::Gba, None)
            .unwrap()
            .0
    } else {
        compile_project_full(&path, path.parent(), &[], true).unwrap().0
    }
}

const KEYS_SRC: &str = "\
interface P { x: number; y: number }
let p: P = { x: 1, y: 2 }
console.log(Object.keys(p).join(\",\"))
";

#[test]
fn gba_object_keys_on_struct_is_native_field_names() {
    let rust = emit(KEYS_SRC, "gba_keys.tish", true);
    // Field names emitted as a native array literal — no boxed `tish_object_keys` call on the struct.
    assert!(
        rust.contains("Value::String(\"x\".into()), Value::String(\"y\".into())"),
        "GBA `Object.keys(struct)` must emit the field names natively (no struct box / hashmap).\n{rust}"
    );
}

#[test]
fn desktop_object_keys_stays_boxed_untouched() {
    let rust = emit(KEYS_SRC, "desk_keys.tish", false);
    assert!(
        !rust.contains("Value::String(\"x\".into()), Value::String(\"y\".into())"),
        "desktop reflection must stay on the boxed path (GBA-gated native lowering leaked to desktop).\n{rust}"
    );
}

const SPREAD_SRC: &str = "\
interface P { x: number; y: number }
let base: P = { x: 1, y: 2 }
let merged = { ...base, z: 3 }
console.log(JSON.stringify(merged))
";

#[test]
fn gba_struct_spread_inserts_fields_natively() {
    let rust = emit(SPREAD_SRC, "gba_spread.tish", true);
    // Native per-field inserts from a borrowed struct — not the boxed `object_spread_props` helper
    // (that stays the fallback for a struct hidden behind a `Value`) and not a hashmap materialisation.
    assert!(
        rust.contains("let _s = &(") && rust.contains("_obj.insert("),
        "GBA `{{ ...struct }}` must insert the struct's fields natively.\n{rust}"
    );
}

#[test]
fn desktop_struct_spread_stays_boxed_untouched() {
    let rust = emit(SPREAD_SRC, "desk_spread.tish", false);
    // Desktop must keep the inline `if let Value::Object` clone-shape path — the native `let _s = &(`
    // insert form must NOT appear (that would mean the GBA lowering leaked onto the object_spread bench).
    assert!(
        !rust.contains("let _s = &("),
        "desktop struct spread must stay boxed/inline (bench-safe); native inserts leaked.\n{rust}"
    );
}

const IN_SRC: &str = "\
interface P { x: number; y: number }
let p: P = { x: 1, y: 2 }
console.log(\"x\" in p)
console.log(\"z\" in p)
";

#[test]
fn gba_in_on_struct_folds_to_native_bool() {
    let rust = emit(IN_SRC, "gba_in.tish", true);
    // A string-literal key `in` a known struct folds to a compile-time bool — no boxed `tish_in_operator`
    // on the struct, no `Value::Struct` box.
    assert!(
        rust.contains("Value::Bool(true)") && rust.contains("Value::Bool(false)"),
        "GBA `\"x\" in struct` must fold to a compile-time bool.\n{rust}"
    );
}

#[test]
fn desktop_in_on_struct_stays_boxed_untouched() {
    let rust = emit(IN_SRC, "desk_in.tish", false);
    assert!(
        rust.contains("tish_in_operator"),
        "desktop `in` must stay on the boxed runtime operator (GBA native lowering leaked).\n{rust}"
    );
}
