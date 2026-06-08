//! End-to-end B2 validation: build the fixture cdylib, `load_module` it, and call its exports
//! through the wrapped `Value::native` shims — exercising the whole load → register → marshal path
//! with a real `dlopen`'d artifact (not a same-binary function pointer).

#![cfg(not(target_arch = "wasm32"))]

use std::path::PathBuf;
use std::process::Command;

use tishlang_core::Value;

fn build_and_locate_fixture() -> PathBuf {
    let manifest = format!(
        "{}/tests/fixtures/testmod/Cargo.toml",
        env!("CARGO_MANIFEST_DIR")
    );
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let status = Command::new(cargo)
        .args(["build", "--release", "--manifest-path", &manifest])
        .status()
        .expect("spawn cargo to build fixture cdylib");
    assert!(status.success(), "fixture cdylib build failed");

    let dir = format!(
        "{}/tests/fixtures/testmod/target/release",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("read {dir}: {e}"))
        .filter_map(|e| e.ok().map(|e| e.path()))
        .find(|p| {
            let name = p.file_name().unwrap_or_default().to_string_lossy();
            name.contains("tish_ffi_testmod")
                && matches!(
                    p.extension().and_then(|x| x.to_str()),
                    Some("dylib") | Some("so") | Some("dll")
                )
        })
        .expect("built cdylib artifact (lib*.{dylib,so} / *.dll)")
}

#[test]
fn load_and_call_real_cdylib() {
    let lib = build_and_locate_fixture();
    let module = tishlang_ffi::load_module(lib.to_str().unwrap())
        .unwrap_or_else(|e| panic!("load_module: {e}"));

    // `triple(7) === 21`
    match module.get("triple") {
        Some(Value::Function(f)) => match f(&[Value::Number(7.0)]) {
            Value::Number(n) => assert_eq!(n, 21.0),
            other => panic!("triple(7) = {other:?}"),
        },
        other => panic!("triple export = {other:?}"),
    }

    // `make_pair(1, 2)` builds an array of length 2 inside the extension, via the C ABI.
    match module.get("make_pair") {
        Some(Value::Function(f)) => match f(&[Value::Number(1.0), Value::Number(2.0)]) {
            Value::Array(a) => assert_eq!(a.borrow().len(), 2),
            other => panic!("make_pair = {other:?}"),
        },
        other => panic!("make_pair export = {other:?}"),
    }
}
