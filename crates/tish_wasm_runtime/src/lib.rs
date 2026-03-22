//! Tish VM as a WebAssembly module.
//!
//! Two targets:
//! - **Browser** (wasm32-unknown-unknown): use `--features browser`, wasm-bindgen, console output
//! - **WASI/Wasmtime** (wasm32-wasi): no features, println! via WASI, run with `wasmtime app.wasm`

use tishlang_bytecode::deserialize;
use tishlang_vm::Vm;

/// Run serialized Tish bytecode (WASI/Wasmtime or native).
///
/// `chunk` is the output of `tishlang_bytecode::serialize(chunk)`.
/// Uses println! for output (WASI fd_write when built for wasm32-wasi).
#[cfg(not(feature = "browser"))]
pub fn run_wasi(chunk: &[u8]) -> Result<(), String> {
    let chunk = deserialize(chunk)?;
    let mut vm = Vm::new();
    vm.run(&chunk)?;
    Ok(())
}

/// Run serialized Tish bytecode in the browser.
///
/// `chunk` is the output of `tishlang_bytecode::serialize(chunk)`.
/// Errors are returned as a JsValue (string).
#[cfg(feature = "browser")]
use wasm_bindgen::prelude::*;

#[cfg(feature = "browser")]
#[wasm_bindgen]
pub fn run(chunk: Vec<u8>) -> Result<(), JsValue> {
    let chunk = deserialize(&chunk).map_err(|e| JsValue::from_str(&e))?;
    let mut vm = Vm::new();
    vm.run(&chunk).map_err(|e| JsValue::from_str(&e))?;
    Ok(())
}
