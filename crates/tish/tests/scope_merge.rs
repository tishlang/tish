//! Regression for #38: a merged `@scope/pkg` package (declared via `tish.module`, `exports.tish`,
//! etc.) must resolve as merged **Tish source**, not be routed to the native Rust-crate path (which
//! would reject it with "not a Tish native module" or mis-treat it as a crate). Native `@scope`
//! crates (marked with `tish.crate` / `tish.rustDependencies`) remain native and are exercised by
//! the downstream regression (tish-apple/tish-macos).

use std::fs;

use tishlang_ast::{Expr, Statement};
use tishlang_compile::{merge_modules, resolve_project};

/// Build a tempdir project whose `main.tish` imports a merged `@test/greet` package. `tish_field`
/// is the extra package.json snippet declaring how the package presents itself (`,"tish":{…}` etc.).
fn setup(tish_field: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let pkg = root.join("node_modules/@test/greet");
    fs::create_dir_all(&pkg).unwrap();
    fs::write(
        pkg.join("package.json"),
        format!(r#"{{"name":"@test/greet","version":"1.0.0"{tish_field}}}"#),
    )
    .unwrap();
    fs::write(pkg.join("index.tish"), "export fn greet() { return \"hi\" }\n").unwrap();
    fs::write(
        root.join("main.tish"),
        "import { greet } from \"@test/greet\"\nconsole.log(greet())\n",
    )
    .unwrap();
    dir
}

fn assert_merged_as_source(dir: &tempfile::TempDir) {
    let root = dir.path();
    let modules = resolve_project(&root.join("main.tish"), Some(root)).expect("resolve");
    let merged = merge_modules(modules).expect("merge");
    // Merged as source ⇒ the package's `greet` fn is folded into the top-level program.
    let has_greet = merged
        .program
        .statements
        .iter()
        .any(|s| matches!(s, Statement::FunDecl { name, .. } if name.as_ref() == "greet"));
    assert!(
        has_greet,
        "merged @scope package's fn must be folded into the program (merged as Tish source)"
    );
    // ...and NOT lowered to a native module load (the #38 mis-routing).
    let native_scope = merged.program.statements.iter().any(|s| {
        matches!(
            s,
            Statement::VarDecl { init: Some(Expr::NativeModuleLoad { spec, .. }), .. }
            if spec.contains("@test")
        )
    });
    assert!(
        !native_scope,
        "a merged @scope package must NOT be routed to the native Rust-crate path (#38)"
    );
}

#[test]
fn scope_tish_module_true_merges_as_source() {
    assert_merged_as_source(&setup(r#","tish":{"module":true}"#));
}

#[test]
fn scope_tish_module_string_path_merges_as_source() {
    assert_merged_as_source(&setup(r#","tish":{"module":"index.tish"}"#));
}

#[test]
fn scope_exports_tish_merges_as_source() {
    assert_merged_as_source(&setup(r#","exports":{"tish":"./index.tish"}"#));
}

#[test]
fn scope_plain_main_merges_as_source() {
    assert_merged_as_source(&setup(r#","main":"index.tish""#));
}
