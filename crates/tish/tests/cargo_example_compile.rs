//! `cargo:` + `tish.rustDependencies` using the in-repo fixture at `tests/fixtures/cargo_example_project/`
//! (same layout as the standalone `tish-cargo-example` template).

use std::path::PathBuf;

use tishlang_ast::Statement;
use tishlang_compile::{compile_project_full, merge_modules, resolve_project};

fn native_build_features_from_cli(cli_features: &[String]) -> Vec<String> {
    if cli_features.is_empty() {
        let mut v: Vec<String> = tishlang_vm::all_compiled_capabilities()
            .into_iter()
            .collect();
        v.sort();
        v
    } else {
        cli_features.to_vec()
    }
}

#[test]
fn resolve_and_merge_cargo_example_fixture() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let input_path = manifest_dir
        .join("tests/fixtures/cargo_example_project/src/main.tish")
        .canonicalize()
        .expect("cargo_example_project test fixture");
    let project_root = input_path.parent().map(|p| {
        if p.file_name().and_then(|n| n.to_str()) == Some("src") {
            p.parent().unwrap_or(p)
        } else {
            p
        }
    });
    let modules = resolve_project(&input_path, project_root).unwrap();
    assert_eq!(modules.len(), 1, "expected single entry module");
    let first = &modules[0].program.statements[0];
    let Statement::Import { from, .. } = first else {
        panic!("expected import, got {:?}", first);
    };
    assert_eq!(from.as_ref(), "cargo:demo_shim");
    merge_modules(modules).unwrap();
}

#[test]
fn compile_project_full_cargo_example_fixture() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let example_main = manifest_dir
        .join("tests/fixtures/cargo_example_project/src/main.tish")
        .canonicalize()
        .expect("cargo_example_project test fixture");
    let input_path = example_main;
    let project_root = input_path.parent().map(|p| {
        if p.file_name().and_then(|n| n.to_str()) == Some("src") {
            p.parent().unwrap_or(p)
        } else {
            p
        }
    });
    let features = native_build_features_from_cli(&[]);
    let r = compile_project_full(&input_path, project_root, &features, true);
    assert!(
        r.is_ok(),
        "compile_project_full failed: {:?}",
        r.map_err(|e| e.message)
    );
}
