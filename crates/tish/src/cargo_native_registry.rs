//! Registers `cargo:*` Rust shims on the bytecode VM (`tish run`, REPL).
//!
//! Native `tish build` outputs register their own crates at link time; the
//! interpreter path must populate [`tishlang_vm::Vm::register_native_module`].

#[cfg(feature = "pg")]
pub(crate) fn register_bytecode_native_modules(vm: &mut tishlang_vm::Vm) {
    use std::sync::Arc;
    use tishlang_core::{ObjectMap, Value};

    let mut om = ObjectMap::with_capacity(8);
    om.insert(
        Arc::from("per_worker_client"),
        Value::native(tishlang_pg::per_worker_client),
    );
    om.insert(Arc::from("connect"), Value::native(tishlang_pg::connect));
    om.insert(Arc::from("prepare"), Value::native(tishlang_pg::prepare));
    om.insert(
        Arc::from("query_prepared"),
        Value::native(tishlang_pg::query_prepared),
    );
    om.insert(
        Arc::from("query_all"),
        Value::native(tishlang_pg::query_all),
    );
    om.insert(Arc::from("migrate"), Value::native(tishlang_pg::migrate));
    om.insert(Arc::from("close"), Value::native(tishlang_pg::close));
    vm.register_native_module("cargo:tish_pg", om);
}

#[cfg(not(feature = "pg"))]
pub(crate) fn register_bytecode_native_modules(_vm: &mut tishlang_vm::Vm) {}
