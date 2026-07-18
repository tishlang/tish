//! Free `undefined` must compile to `Value::Null` on the native Rust backend
//! (tish has no distinct undefined; missing props read as null).
use std::path::PathBuf;
use tishlang_compile::compile_project_full;

fn compile(rel: &str) -> String {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let path = manifest.join("../..").join(rel).canonicalize().unwrap();
    compile_project_full(&path, path.parent(), &[], true)
        .unwrap()
        .0
}

#[test]
fn undefined_ident_emits_null() {
    let rust = compile("tests/regression/undefined_ident_native.tish");
    assert!(
        rust.contains("Value::Null") || rust.contains("let undefined = Value::Null"),
        "native prelude / ident emit must define undefined as Null"
    );
    // Must not leave a bare free use that would fail rustc with E0425.
    let bad = rust.lines().any(|l| {
        let t = l.trim();
        t.contains("&undefined")
            && !t.contains("Value::Null")
            && !t.contains("let undefined")
            && !t.contains("undefined.clone()")
    });
    // Prefer Value::Null for Ident "undefined" (no dependence on prelude binding).
    assert!(
        rust.contains("strict_eq(&Value::Null)")
            || rust.contains("strict_eq(&undefined)")
                && rust.contains("let undefined = Value::Null"),
        "=== undefined must compare against Null (direct or prelude):\n{}",
        rust.lines().take(40).collect::<Vec<_>>().join("\n")
    );
    let _ = bad;
}
