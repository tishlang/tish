//! `--target rust-lib` (`NativeEmitMode::RustLib`): a Tish library compiled to a Rust library crate
//! whose public API is the entry module's `export fn`s — the Rust counterpart of `--target js`.
//!
//! The module body still lowers to the ordinary whole-program `run()`. What the library emit adds is
//! (a) one `pub fn` per export and (b) keeping the exported bindings alive past `run()` in a
//! thread-local, so module-level state survives between calls. These tests pin that emitted shape;
//! the crate layout and the behavioural end (state persists, reassignment is observable, init runs
//! once) are covered in `crates/tish/tests/rust_lib_crate.rs`.
use std::path::PathBuf;
use tishlang_compile::{compile_project_full, compile_project_full_emit, NativeEmitMode};

fn fixture(rel: &str) -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest.join("../..").join(rel).canonicalize().unwrap()
}

fn compile_lib(rel: &str) -> String {
    let path = fixture(rel);
    compile_project_full_emit(&path, path.parent(), &[], true, NativeEmitMode::RustLib, None)
        .expect("rust-lib emit")
        .0
}

#[test]
fn every_export_becomes_an_arity_matched_pub_fn() {
    let rust = compile_lib("tests/regression/rust_lib_emit.tish");
    for sig in [
        "pub fn register(a0: Value, a1: Value) -> Value",
        "pub fn lookup(a0: Value) -> Value",
        "pub fn clearAll() -> Value",
        "pub fn initRuns() -> Value",
        "pub fn add(a0: Value, a1: Value) -> Value",
    ] {
        assert!(
            rust.contains(sig),
            "missing `{sig}` in the emitted library surface:\n{rust}"
        );
    }
}

#[test]
fn library_crate_has_no_binary_entry_point() {
    let rust = compile_lib("tests/regression/rust_lib_emit.tish");
    // A `fn main` (or the iOS C entry) would make this an executable/staticlib, not an rlib.
    assert!(
        !rust.contains("fn main()"),
        "a library crate must not emit a binary entry point:\n{rust}"
    );
    assert!(
        !rust.contains("tish_ios_launch"),
        "the iOS staticlib entry belongs to EmbeddedLib, not RustLib:\n{rust}"
    );
}

#[test]
fn exports_are_published_out_of_run_so_module_state_outlives_it() {
    let rust = compile_lib("tests/regression/rust_lib_emit.tish");
    // Without this call the closures — and the `VmRef` cells they captured — drop with `run()`'s
    // frame, and every `pub fn` would see a fresh module on each call.
    assert!(
        rust.contains("__tish_publish_exports(::std::vec!["),
        "run() must hand its exported bindings to the module table:\n{rust}"
    );
    assert!(
        rust.contains("(\"clearAll\", clearAll.clone())"),
        "each export must be published under its exported name:\n{rust}"
    );
    // `Value` is `!Send` unless `send-values` (which follows `http`) is on, so a `static` would not
    // compile for the common configuration.
    assert!(
        rust.contains("thread_local! {") && rust.contains("static __TISH_MODULE"),
        "module storage must be thread-local:\n{rust}"
    );
}

#[test]
fn runtime_prelude_is_re_exported_so_consumers_cannot_version_skew_value() {
    let rust = compile_lib("tests/regression/rust_lib_emit.tish");
    assert!(
        rust.contains("pub use tishlang_runtime::{console_debug"),
        "RustLib must re-export the runtime prelude (so `Value` comes from THIS crate):\n{rust}"
    );
    assert!(
        rust.contains("pub use tishlang_runtime as runtime;"),
        "RustLib must re-export the runtime module:\n{rust}"
    );
}

#[test]
fn other_emit_modes_keep_the_prelude_private_and_gain_no_surface() {
    let path = fixture("tests/regression/rust_lib_emit.tish");
    let rust = compile_project_full(&path, path.parent(), &[], true)
        .expect("desktop emit")
        .0;
    assert!(
        !rust.contains("__TISH_MODULE") && !rust.contains("pub fn clearAll"),
        "the library surface must be RustLib-only:\n{rust}"
    );
    assert!(
        rust.contains("use tishlang_runtime::{console_debug")
            && !rust.contains("pub use tishlang_runtime::{console_debug"),
        "a binary must keep the runtime prelude private:\n{rust}"
    );
}

#[test]
fn a_value_export_is_rejected_rather_than_silently_returning_null() {
    // A value export has no arity and its Rust local need not even be a `Value` (a `const` string
    // lowers to `&str`), so it cannot be published like a closure. Fail loudly instead.
    let dir = std::env::temp_dir().join("tish_rustlib_value_export");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("index.tish");
    std::fs::write(&path, "export let answer = 42\nexport fn get() { return answer }\n").unwrap();
    let err = compile_project_full_emit(
        &path,
        Some(dir.as_path()),
        &[],
        true,
        NativeEmitMode::RustLib,
        None,
    )
    .expect_err("a value export must be rejected");
    assert!(
        err.message.contains("only `export fn`") && err.message.contains("answer"),
        "the error should name the offending export: {}",
        err.message
    );
}

#[test]
fn an_entry_module_with_no_exports_is_rejected() {
    let dir = std::env::temp_dir().join("tish_rustlib_no_exports");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("index.tish");
    std::fs::write(&path, "fn unused() { return 1 }\n").unwrap();
    let err = compile_project_full_emit(
        &path,
        Some(dir.as_path()),
        &[],
        true,
        NativeEmitMode::RustLib,
        None,
    )
    .expect_err("a crate with no public API must be rejected");
    assert!(
        err.message.contains("exports nothing"),
        "unexpected error: {}",
        err.message
    );
}
