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

/// The other half of the #38 split: a `@scope/pkg` that opts in via `tish.crate` (the tish-apple
/// hosts, published as `@tishlang/tish-macos` / `@tishlang/tish-ios`) routes to the NATIVE path —
/// lowered to `NativeModuleLoad`, not merged as source.
#[test]
fn scope_tish_crate_routes_native() {
    let dir = setup(r#","tish":{"module":true,"crate":"test-greet","export":"greet_object"}"#);
    let root = dir.path();
    let modules = resolve_project(&root.join("main.tish"), Some(root)).expect("resolve");
    let merged = merge_modules(modules).expect("merge");
    let native_scope = merged.program.statements.iter().any(|s| {
        matches!(
            s,
            Statement::VarDecl { init: Some(Expr::NativeModuleLoad { spec, .. }), .. }
            if spec.as_ref() == "@test/greet"
        )
    });
    assert!(
        native_scope,
        "@scope package with tish.crate must lower to NativeModuleLoad (native Rust-crate path)"
    );
    let has_greet = merged
        .program
        .statements
        .iter()
        .any(|s| matches!(s, Statement::FunDecl { name, .. } if name.as_ref() == "greet"));
    assert!(
        !has_greet,
        "@scope package with tish.crate must NOT be merged as Tish source"
    );
}

// ── #587: two modules exporting the SAME name, flattened into one scope ──────────────────────────
//
// `isolate_private_top_level_bindings` renamed colliding PRIVATE top-level names but skipped
// exported ones, on the reasoning that "exported names stay stable so imports keep resolving".
// `module_exports` is an indirection from export name to source symbol, so that was stricter than
// necessary — and leaving the collision produced a silently WRONG program rather than a slower one.
//
// The barrel pattern is where it bites: `import { greet as _greet }` + a local `export fn greet`
// that calls `_greet`. Flattened, that emits two `function greet` declarations, and because JS
// function declarations HOIST, the second overwrote the first BEFORE `const _greet = greet` ran.
// `_greet` then pointed at the wrapper and the program recursed until the stack blew.
//
// The entry module is excluded from renaming on purpose: its exports are the bundle's public
// surface, so moving those would change the API the caller sees. Only the dep is renamed here.

/// dep exports `greet`; barrel imports it aliased and re-exports its own `greet` wrapper.
fn setup_barrel() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir_all(root.join("dep")).unwrap();
    fs::create_dir_all(root.join("app")).unwrap();
    fs::write(
        root.join("dep/lib.tish"),
        "export fn greet(name) { return \"hi \" + name }\n",
    )
    .unwrap();
    fs::write(
        root.join("app/barrel.tish"),
        "import { greet as _greet } from \"../dep/lib.tish\"\n\
         export fn greet(name) { return _greet(name) }\n",
    )
    .unwrap();
    fs::write(
        root.join("main.tish"),
        "import { greet } from \"./app/barrel.tish\"\nconsole.log(greet(\"world\"))\n",
    )
    .unwrap();
    dir
}

#[test]
fn colliding_exported_names_are_isolated_across_modules() {
    let dir = setup_barrel();
    let root = dir.path();
    let modules = resolve_project(&root.join("main.tish"), Some(root)).expect("resolve");
    let merged = merge_modules(modules).expect("merge");

    // The two `greet` declarations must not both be named `greet` in the flat program — that is the
    // collision, and on the JS target hoisting silently resolves it the wrong way.
    let greet_decls: Vec<&str> = merged
        .program
        .statements
        .iter()
        .filter_map(|s| match s {
            Statement::FunDecl { name, .. } => Some(name.as_ref()),
            _ => None,
        })
        .filter(|n| n.starts_with("greet"))
        .collect();
    assert!(
        greet_decls.len() >= 2,
        "expected both modules' greet fns in the merged program, got {:?}",
        greet_decls
    );
    let bare = greet_decls.iter().filter(|n| **n == "greet").count();
    assert!(
        bare <= 1,
        "two top-level `greet` declarations survived the merge — on the JS target the later one \
         hoists over the former and the barrel's alias captures the wrapper (infinite recursion). \
         Got {:?}",
        greet_decls
    );

    // The alias must bind to the DEP's symbol, not to the local wrapper.
    let alias_target = merged.program.statements.iter().find_map(|s| match s {
        Statement::VarDecl {
            name,
            init: Some(Expr::Ident { name: src, .. }),
            ..
        } if name.as_ref() == "_greet" => Some(src.to_string()),
        _ => None,
    });
    if let Some(target) = alias_target {
        assert_ne!(
            target, "greet",
            "`_greet` must not bind to the ambiguous bare `greet`; it should name the dep's symbol"
        );
    }
}
