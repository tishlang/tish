//! #295: `tish build --target js --format bundle` must re-emit the entry module's named exports as a
//! real ES `export { … }` (previously only `export default` surfaced; `export fn/const/{…}` were
//! silently dropped, forcing downstream regex post-processing of every bundle).

use std::io::Write;

#[test]
fn bundle_emits_named_exports() {
    // CARGO_TARGET_TMPDIR is a per-test-binary scratch dir under `target/` (no `std::env::temp_dir`).
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("bundle_exports_295");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("entry.tish");
    let src = "export fn mountFoo(h) { return h }\n\
               export const VERSION = \"1\"\n\
               fn helper() { return 2 }\n\
               export { helper as helperExported }\n\
               export default 42\n";
    let mut f = std::fs::File::create(&path).unwrap();
    f.write_all(src.as_bytes()).unwrap();
    f.sync_all().unwrap();
    drop(f);

    let js = tishlang_compile_js::compile_project_with_jsx(&path, Some(&dir), false)
        .expect("compile_project_with_jsx failed");

    assert!(
        js.contains("export { VERSION, helper as helperExported, mountFoo };"),
        "bundle must emit named exports, got:\n{}",
        &js[js.len().saturating_sub(300)..]
    );
    assert!(
        js.contains("export default __default_0;"),
        "bundle must still emit the default export"
    );

    let _ = std::fs::remove_file(&path);
}
