//! ABI v2 end to end: a cdylib that links no host symbols receives the `TishHostApi` table via
//! `tish_module_register_v2`, and objects, callbacks (both directions) and throws cross it.

#![cfg(not(target_arch = "wasm32"))]

use std::path::PathBuf;
use std::process::Command;

use tishlang_core::{take_pending_throw, ObjectData, Value, VmRef};

fn build_and_locate_fixture() -> PathBuf {
    let manifest = format!(
        "{}/tests/fixtures/testmod_v2/Cargo.toml",
        env!("CARGO_MANIFEST_DIR")
    );
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let status = Command::new(cargo)
        .args(["build", "--release", "--manifest-path", &manifest])
        .status()
        .expect("spawn cargo to build v2 fixture cdylib");
    assert!(status.success(), "v2 fixture cdylib build failed");

    let dir = format!(
        "{}/tests/fixtures/testmod_v2/target/release",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("read {dir}: {e}"))
        .filter_map(|e| e.ok().map(|e| e.path()))
        .find(|p| {
            let name = p.file_name().unwrap_or_default().to_string_lossy();
            name.contains("tish_ffi_testmod_v2")
                && matches!(
                    p.extension().and_then(|x| x.to_str()),
                    Some("dylib") | Some("so") | Some("dll")
                )
        })
        .expect("built v2 cdylib artifact")
}

fn export(name: &str) -> tishlang_core::NativeFn {
    let lib = build_and_locate_fixture();
    let module = tishlang_ffi::load_module(lib.to_str().unwrap())
        .unwrap_or_else(|e| panic!("load_module: {e}"));
    match module.get(name) {
        Some(Value::Function(f)) => f.clone(),
        other => panic!("{name} export = {other:?}"),
    }
}

fn obj(pairs: &[(&str, Value)]) -> Value {
    let mut o = ObjectData::default();
    for (k, v) in pairs {
        o.strings.insert((*k).into(), v.clone());
    }
    Value::Object(VmRef::new(o))
}

#[test]
fn host_object_keys_are_readable_in_module() {
    let describe = export("describe");
    let out = describe.call(&[obj(&[("title", Value::String("Safari".into())), ("rank", Value::Number(1.0))])]);
    match out {
        Value::String(s) => assert_eq!(s.as_str(), "title,rank|2"),
        other => panic!("describe = {other:?}"),
    }
}

#[test]
fn module_calls_host_function() {
    let apply = export("apply");
    let double = Value::native(|a: &[Value]| match a.first() {
        Some(Value::Number(n)) => Value::Number(n * 2.0),
        _ => Value::Null,
    });
    match apply.call(&[double, Value::Number(20.0)]) {
        Value::Number(n) => assert_eq!(n, 41.0),
        other => panic!("apply = {other:?}"),
    }
}

#[test]
fn host_calls_module_function() {
    let make_adder = export("make_adder");
    let add5 = make_adder.call(&[Value::Number(5.0)]);
    let Value::Function(add5) = add5 else {
        panic!("make_adder returned {add5:?}");
    };
    match add5.call(&[Value::Number(10.0)]) {
        Value::Number(n) => assert_eq!(n, 15.0),
        other => panic!("add5(10) = {other:?}"),
    }
}

#[test]
fn module_throw_reaches_host() {
    let boom = export("boom");
    let _ = take_pending_throw();
    let out = boom.call(&[Value::String("plugin failed".into())]);
    assert!(matches!(out, Value::Null));
    match take_pending_throw() {
        Some(Value::String(s)) => assert_eq!(s.as_str(), "plugin failed"),
        other => panic!("pending throw = {other:?}"),
    }
}

#[test]
fn host_throw_inside_callback_propagates_through_module() {
    let apply = export("apply");
    let thrower = Value::native(|_a: &[Value]| {
        tishlang_core::set_pending_throw(Value::String("host callback failed".into()));
        Value::Null
    });
    let _ = take_pending_throw();
    let out = apply.call(&[thrower, Value::Number(1.0)]);
    assert!(matches!(out, Value::Null));
    match take_pending_throw() {
        Some(Value::String(s)) => assert_eq!(s.as_str(), "host callback failed"),
        other => panic!("pending throw = {other:?}"),
    }
}

#[test]
fn nested_values_and_nul_strings_round_trip() {
    let echo = export("echo");
    let input = obj(&[
        ("name", Value::String("a\0b".into())),
        (
            "items",
            Value::Array(VmRef::new(vec![
                Value::Number(1.0),
                obj(&[("ok", Value::Bool(true))]),
                Value::Null,
            ])),
        ),
    ]);
    let out = echo.call(&[input]);
    let Value::Object(o) = out else {
        panic!("echo = {out:?}");
    };
    let o = o.borrow();
    match o.strings.get("name") {
        Some(Value::String(s)) => assert_eq!(s.as_str(), "a\0b"),
        other => panic!("name = {other:?}"),
    }
    match o.strings.get("items") {
        Some(Value::Array(a)) => {
            let a = a.borrow();
            assert_eq!(a.len(), 3);
            match &a[1] {
                Value::Object(inner) => {
                    assert!(matches!(inner.borrow().strings.get("ok"), Some(Value::Bool(true))))
                }
                other => panic!("items[1] = {other:?}"),
            }
        }
        other => panic!("items = {other:?}"),
    }
}
