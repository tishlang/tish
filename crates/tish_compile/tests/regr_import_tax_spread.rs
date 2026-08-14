//! Spread calls still need a Value binding when the callee is not routed to `_bfn`
//! (e.g. M5-eligible numeric fns). Stripping the twin caused E0425 on sum3/add4.
use std::path::PathBuf;
use tishlang_compile::compile_project_full;

#[test]
fn spread_call_keeps_or_routes_callee() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/regr_import_tax_spread");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("main.tish");
    std::fs::write(
        &path,
        r#"
fn sum3(a, b, c) { return a + b + c }
let nums = [1, 2, 3]
console.log(sum3(...nums) === 6)
"#,
    )
    .unwrap();
    let path = path.canonicalize().unwrap();
    let (rust, _, _, _) = compile_project_full(&path, path.parent(), &[], true).unwrap();
    let has_binding = rust.contains("let sum3 = {") || rust.contains("sum3_cell");
    let has_bfn = rust.contains("fn sum3_bfn(") && rust.contains("sum3_bfn(");
    assert!(
        has_binding || has_bfn,
        "spread callee must keep a Value binding or route to _bfn:\n{rust}"
    );
    assert!(
        !rust.contains("(sum3).clone()") || has_binding,
        "value_call on sum3 requires the binding:\n{rust}"
    );
}
