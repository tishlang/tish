//! Empty typed arrays must emit `Vec::<T>::new()`, not bare `vec![]` (E0282 when unused after DCE).
use std::path::PathBuf;
use tishlang_compile::compile_project_full;

#[test]
fn empty_typed_struct_array_has_turbofish() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/regr_empty_typed_vec");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("main.tish");
    std::fs::write(
        &path,
        r#"
type LNode = { kind: number }
let LN: LNode[] = []
console.log(0)
"#,
    )
    .unwrap();
    let path = path.canonicalize().unwrap();
    let (rust, _, _, _) = compile_project_full(&path, path.parent(), &[], true).unwrap();
    assert!(
        rust.contains("Vec::<") && rust.contains("::new()"),
        "empty LNode[] must use Vec::<T>::new():\n{rust}"
    );
    assert!(
        !rust.contains("VmRef::new(vec![])") && !rust.contains("= vec![]"),
        "must not emit bare vec![]:\n{rust}"
    );
}
