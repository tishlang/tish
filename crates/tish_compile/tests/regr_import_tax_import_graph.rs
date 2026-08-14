//! Import tax across merge_modules: unused exports must not become Value::native.
use std::path::PathBuf;
use tishlang_compile::compile_project_full;

fn write(dir: &std::path::Path, name: &str, src: &str) -> PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, src).unwrap();
    p
}

#[test]
fn unused_exports_of_imported_module_are_not_heap_closures() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/regr_import_tax_graph");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    write(
        &dir,
        "lib.tish",
        r#"
export function deadA() { return 1 }
export function deadB() { return 2 }
export function deadC() { return 3 }
export function useA() { return deadA() }
export function live() { return 42 }
"#,
    );
    let main = write(
        &dir,
        "main.tish",
        r#"
import { live } from './lib.tish'
console.log(live())
"#,
    );
    let main = main.canonicalize().unwrap();
    let (rust, _, _, _) = compile_project_full(&main, main.parent(), &[], true).unwrap();
    assert!(
        rust.contains("let live = {") || rust.contains("fn live_"),
        "live must remain:\n{rust}"
    );
    for dead in ["deadA", "deadB", "deadC", "useA"] {
        assert!(
            !rust.contains(&format!("let {dead} = {{")),
            "{dead} must not be a Value::native binding:\n{rust}"
        );
        assert!(
            !rust.contains(&format!("{dead}_cell")),
            "{dead} must not get a VmRef cell:\n{rust}"
        );
    }
}

#[test]
fn unused_exports_stay_dead_when_sibling_is_used() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/regr_import_tax_bfn");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    write(
        &dir,
        "pkg.tish",
        r#"
export function dead1() { return "d" }
export function helper() { return "h" }
export function uiInit() { return helper() + "!" }
export function ui_begin() { return "u" }
"#,
    );
    let main = write(
        &dir,
        "main.tish",
        r#"
import { uiInit } from './pkg.tish'
console.log(uiInit())
"#,
    );
    let main = main.canonicalize().unwrap();
    let (rust, _, _, _) = compile_project_full(&main, main.parent(), &[], true).unwrap();
    assert!(
        !rust.contains("let dead1 = {") && !rust.contains("let ui_begin = {"),
        "unused exports must be omitted:\n{rust}"
    );
    assert!(
        rust.contains("uiInit_bfn(") || rust.contains("let uiInit = {") || rust.contains("uiInit_native("),
        "uiInit must remain callable:\n{rust}"
    );
}
