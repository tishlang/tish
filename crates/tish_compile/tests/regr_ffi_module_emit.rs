//! `--crate-type cdylib` (`NativeEmitMode::FfiModule`): the RustLib surface plus a `tish_ffi` ABI v2
//! entry point, so the result can be loaded at runtime via `import { .. } from "ffi:./x.lib"`.
//! The loader behaviour itself is covered by `crates/tish_ffi/tests/loader_v2.rs`.
use std::path::{Path, PathBuf};
use tishlang_compile::{compile_project_full_emit, CompileError, NativeEmitMode};

fn compile(path: &Path, mode: NativeEmitMode) -> Result<String, CompileError> {
    compile_project_full_emit(path, path.parent(), &[], true, mode, None).map(|r| r.0)
}

fn fixture(rel: &str) -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest.join("../..").join(rel).canonicalize().unwrap()
}

fn temp_module(name: &str, src: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(name);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("index.tish");
    std::fs::write(&path, src).unwrap();
    path
}

#[test]
fn ffi_module_registers_every_export_by_name() {
    let rust = compile(&fixture("tests/regression/rust_lib_emit.tish"), NativeEmitMode::FfiModule)
        .expect("ffi module emit");
    assert!(
        rust.contains("pub unsafe extern \"C\" fn tish_module_register_v2("),
        "missing the v2 entry point:\n{rust}"
    );
    for name in ["register", "lookup", "clearAll", "initRuns", "add"] {
        assert!(
            rust.contains(&format!("(b\"{name}\\0\", ")),
            "`{name}` missing from the export table:\n{rust}"
        );
        assert!(
            rust.contains(&format!("__tish_call_export(\"{name}\", &a)")),
            "`{name}` has no trampoline:\n{rust}"
        );
    }
    // Still a library: the Rust API stays available to in-process consumers.
    assert!(rust.contains("pub fn add(a0: Value, a1: Value) -> Value"));
    assert!(!rust.contains("fn main()"));
}

#[test]
fn rust_lib_gains_no_ffi_entry_point() {
    let rust = compile(&fixture("tests/regression/rust_lib_emit.tish"), NativeEmitMode::RustLib)
        .expect("rust-lib emit");
    assert!(
        !rust.contains("tish_module_register_v2") && !rust.contains("tishlang_ffi_guest"),
        "the FFI entry point must be FfiModule-only:\n{rust}"
    );
}

#[test]
fn an_export_named_like_a_generated_item_is_still_exported_over_ffi() {
    let path = temp_module(
        "tish_ffimodule_reserved_name",
        "export fn run(cb) { return cb(1) }\nexport fn other() { return 2 }\n",
    );
    let rust = compile(&path, NativeEmitMode::FfiModule).expect("ffi module emit");
    assert!(
        !rust.contains("pub fn run(a0: Value)"),
        "a `pub fn run` would clash with the module's own `run()`:\n{rust}"
    );
    assert!(
        rust.contains("(b\"run\\0\", ") && rust.contains("__tish_call_export(\"run\", &a)"),
        "`run` must still be reachable by name over FFI:\n{rust}"
    );
}

#[test]
fn tish_ffi_load_module_lowers_to_the_runtime_loader() {
    let path = temp_module(
        "tish_ffi_load_module_emit",
        "import { loadModule } from \"tish:ffi\"\nlet m = loadModule(\"./x.lib\")\nconsole.log(m)\n",
    );
    let rust = compile_project_full_emit(
        &path,
        path.parent(),
        &["ffi".to_string()],
        true,
        NativeEmitMode::DesktopBin,
        None,
    )
    .expect("tish:ffi should compile with the ffi feature")
    .0;
    assert!(
        rust.contains("tishlang_runtime::ffi_load_module(args)"),
        "loadModule must call the runtime loader:\n{rust}"
    );
}

#[test]
fn rust_lib_rejects_an_export_named_like_a_generated_item() {
    let path = temp_module("tish_rustlib_reserved_name", "export fn run() { return 1 }\n");
    let err = compile(&path, NativeEmitMode::RustLib).expect_err("`run` cannot be a Rust pub fn here");
    assert!(
        err.message.contains("clash") && err.message.contains("`run`"),
        "the error should name the clashing export: {}",
        err.message
    );
}

#[test]
fn a_purely_numeric_export_keeps_its_boxed_binding() {
    // `add` is only ever called, and its params are numeric, so the native fast path used to drop
    // its boxed `Value` wrapper. A library publishes every export as a `Value`, so it must stay.
    let path = temp_module(
        "tish_lib_numeric_export",
        "export fn add(a, b) {\n  return a + b\n}\nexport fn greet(name) {\n  return \"hi \" + name\n}\n",
    );
    for mode in [NativeEmitMode::FfiModule, NativeEmitMode::RustLib] {
        let rust = compile(&path, mode).expect("lib emit");
        assert!(rust.contains("(\"add\", add.clone())"), "add is not published:\n{rust}");
        assert!(rust.contains("let add = "), "add's boxed binding was elided ({mode:?}):\n{rust}");
    }
}
